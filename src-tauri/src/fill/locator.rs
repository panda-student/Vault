//! Resolve explicitly named controls locally. Never query control values.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        System::Variant::VARIANT,
        UI::{
            Accessibility::{
                TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_EditControlTypeId,
            },
            WindowsAndMessaging::{
                EnumWindows, GetWindowTextW, IsIconic, IsWindowVisible, SetForegroundWindow,
            },
        },
    },
    core::BOOL,
};

struct Search {
    request: CaptureRequest,
    windows: Vec<isize>,
}
unsafe extern "system" fn enumerate(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let search = unsafe { &mut *(parameter.0 as *mut Search) };
    if matches_window(hwnd, &search.request) {
        search.windows.push(hwnd.0 as isize);
    }
    true.into()
}
fn matches_window(hwnd: HWND, request: &CaptureRequest) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if request.process_id.is_some_and(|id| id != pid) {
            return false;
        }
        if let Some(title) = &request.window_title {
            let mut buffer = [0u16; 2048];
            let count = GetWindowTextW(hwnd, &mut buffer);
            if count <= 0
                || !String::from_utf16_lossy(&buffer[..count as usize])
                    .to_lowercase()
                    .contains(&title.to_lowercase())
            {
                return false;
            }
        }
        true
    }
}
pub(super) fn capture(request: CaptureRequest) -> std::result::Result<Identity, &'static str> {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("accessibility_busy");
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    if std::thread::Builder::new()
        .name("vault-input-locator".into())
        .spawn(move || {
            struct Reset;
            impl Drop for Reset {
                fn drop(&mut self) {
                    BUSY.store(false, Ordering::SeqCst);
                }
            }
            let _reset = Reset;
            let result = locate(request, &worker_cancelled);
            let _ = sender.send(result);
        })
        .is_err()
    {
        BUSY.store(false, Ordering::SeqCst);
        return Err("accessibility_unavailable");
    }
    let result = receiver.recv_timeout(Duration::from_secs(5));
    cancelled.store(true, Ordering::SeqCst);
    result.unwrap_or(Err("accessibility_timeout"))
}
fn locate(
    request: CaptureRequest,
    cancelled: &AtomicBool,
) -> std::result::Result<Identity, &'static str> {
    let _apartment = Apartment::new().map_err(|_| "accessibility_unavailable")?;
    unsafe {
        let original = GetForegroundWindow();
        let target = if request.window_title.is_some() || request.process_id.is_some() {
            let mut search = Search {
                request: request.clone(),
                windows: vec![],
            };
            EnumWindows(Some(enumerate), LPARAM(&mut search as *mut Search as isize))
                .map_err(|_| "window_unavailable")?;
            match search.windows.as_slice() {
                [one] => HWND(*one as *mut _),
                [] => return Err("window_not_found"),
                _ => return Err("ambiguous_window"),
            }
        } else {
            original
        };
        if target.0.is_null() {
            return Err("window_not_found");
        }
        let automation: IUIAutomation2 =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| "accessibility_unavailable")?;
        automation
            .SetConnectionTimeout(500)
            .map_err(|_| "accessibility_unavailable")?;
        automation
            .SetTransactionTimeout(500)
            .map_err(|_| "accessibility_unavailable")?;
        let condition = automation
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_EditControlTypeId.0),
            )
            .map_err(|_| "accessibility_unavailable")?;
        let deadline = Instant::now() + Duration::from_millis(3000);
        loop {
            if cancelled.load(Ordering::SeqCst) {
                return Err("accessibility_timeout");
            }
            // Chromium can invalidate the first UIA tree while enabling accessibility.
            // Retry the entire read-only query, never a partial candidate set.
            let queried = (|| -> std::result::Result<_, &'static str> {
                let root = automation
                    .ElementFromHandle(target)
                    .map_err(|_| "window_unavailable")?;
                let elements = root
                    .FindAll(TreeScope_Descendants, &condition)
                    .map_err(|_| "accessibility_unavailable")?;
                let count = elements.Length().map_err(|_| "accessibility_unavailable")?;
                if count > 4096 {
                    return Err("too_many_controls");
                }
                let mut found = None;
                for index in 0..count {
                    if cancelled.load(Ordering::SeqCst) {
                        return Err("accessibility_timeout");
                    }
                    let element = elements
                        .GetElement(index)
                        .map_err(|_| "accessibility_unavailable")?;
                    let name = element
                        .CurrentName()
                        .map_err(|_| "accessibility_unavailable")?;
                    if name.to_string().trim() != request.field_name.as_deref().unwrap_or("").trim()
                        || element
                            .CurrentIsOffscreen()
                            .map_err(|_| "accessibility_unavailable")?
                            .as_bool()
                    {
                        continue;
                    }
                    if found.is_some() {
                        return Err("ambiguous_field");
                    }
                    found = Some(element);
                }
                Ok(found)
            })();
            let found = match queried {
                Ok(found) => found,
                Err("accessibility_unavailable" | "window_unavailable")
                    if Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                Err(reason) => return Err(reason),
            };
            if let Some(element) = found {
                if !element
                    .CurrentIsEnabled()
                    .map_err(|_| "accessibility_unavailable")?
                    .as_bool()
                {
                    return Err("read_only_or_non_input");
                }
                if let Ok(pattern) =
                    element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    && pattern
                        .CurrentIsReadOnly()
                        .map_err(|_| "accessibility_unavailable")?
                        .as_bool()
                {
                    return Err("read_only_or_non_input");
                }
                let password = element
                    .CurrentIsPassword()
                    .map_err(|_| "accessibility_unavailable")?
                    .as_bool();
                if request
                    .expected_password
                    .is_some_and(|expected| expected != password)
                {
                    return Err("field_type_mismatch");
                }
                if cancelled.load(Ordering::SeqCst) {
                    return Err("accessibility_timeout");
                }
                let foreground = GetForegroundWindow();
                if foreground != original && foreground != target {
                    return Err("focus_changed_during_capture");
                }
                if !matches_window(target, &request) {
                    return Err("window_unavailable");
                }
                if [0x10, 0x11, 0x12, 0x5b, 0x5c]
                    .iter()
                    .any(|key| GetAsyncKeyState(*key) < 0)
                {
                    return Err("modifier_held");
                }
                // Only focus in this worker; it never resolves or types a credential.
                let _ = SetForegroundWindow(target);
                if cancelled.load(Ordering::SeqCst) {
                    return Err("accessibility_timeout");
                }
                element.SetFocus().map_err(|_| "focus_unavailable")?;
                // SetFocus may return before Chromium publishes its new focus state.
                let focus_deadline = Instant::now() + Duration::from_millis(600);
                loop {
                    if cancelled.load(Ordering::SeqCst) {
                        return Err("accessibility_timeout");
                    }
                    let foreground = GetForegroundWindow();
                    if foreground != original && foreground != target {
                        return Err("focus_changed_during_capture");
                    }
                    let focused = automation.GetFocusedElement();
                    let ready = focused.ok().is_some_and(|focused| {
                        automation
                            .CompareElements(&element, &focused)
                            .is_ok_and(|same| same.as_bool())
                            && focused
                                .CurrentHasKeyboardFocus()
                                .is_ok_and(|has| has.as_bool())
                    });
                    if foreground == target && ready {
                        break;
                    }
                    if Instant::now() >= focus_deadline {
                        return Err("focus_unavailable");
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                let (process, runtime, is_password) =
                    accessibility_identity().ok_or("focus_unavailable")?;
                if process == -1 || runtime.is_empty() {
                    return Err("read_only_or_non_input");
                }
                let mut captured = identity(false)?;
                if captured.window != target.0 as isize || cancelled.load(Ordering::SeqCst) {
                    return Err("focus_changed_during_capture");
                }
                captured.process = process;
                captured.runtime = runtime;
                captured.is_password = is_password;
                return Ok(captured);
            }
            if Instant::now() >= deadline {
                return Err("field_not_found");
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
}
