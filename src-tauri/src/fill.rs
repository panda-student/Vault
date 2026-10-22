use serde::{Deserialize, Serialize};
mod locator;

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRequest {
    pub window_title: Option<String>,
    pub process_id: Option<u32>,
    pub field_name: Option<String>,
    pub expected_password: Option<bool>,
}
impl CaptureRequest {
    fn validate(&self) -> crate::error::Result<()> {
        for (field, value, limit) in [
            ("windowTitle", &self.window_title, 512),
            ("fieldName", &self.field_name, 256),
        ] {
            if value.as_ref().is_some_and(|s| {
                s.trim().is_empty() || s.chars().count() > limit || s.chars().any(char::is_control)
            }) {
                return Err(crate::error::AppError::invalid().with(
                    field,
                    "Use a nonempty literal label within the length limit.",
                ));
            }
        }
        if self.process_id == Some(0)
            || ((self.window_title.is_some() || self.process_id.is_some())
                && self.field_name.is_none())
        {
            return Err(crate::error::AppError::invalid().with(
                "fieldName",
                "A window selector requires a named input field; processId must be positive.",
            ));
        }
        Ok(())
    }
}
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use windows::Win32::{
    System::{
        Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        Ole::{
            SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
            SafeArrayUnaccessData,
        },
    },
    UI::{
        Accessibility::{
            CUIAutomation8, IUIAutomation2, IUIAutomationValuePattern, UIA_ButtonControlTypeId,
            UIA_ValuePatternId,
        },
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY,
        },
        WindowsAndMessaging::{
            GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
        },
    },
};
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_secs(30);
#[derive(Clone, PartialEq)]
struct Identity {
    window: isize,
    window_process: u32,
    focus: isize,
    process: i32,
    runtime: Vec<i32>,
    is_password: Option<bool>,
}
struct Target {
    identity: Identity,
    created: Instant,
}
#[derive(Default)]
pub struct Targets {
    items: Mutex<HashMap<String, Target>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillStatus {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_password: Option<bool>,
}
impl FillStatus {
    pub fn status(status: &'static str) -> Self {
        Self {
            status,
            target_id: None,
            expires_in_seconds: None,
            process_id: None,
            reason: None,
            is_password: None,
        }
    }
}
struct Apartment;
impl Apartment {
    fn new() -> std::result::Result<Self, ()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|_| ())?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn identity(use_accessibility: bool) -> std::result::Result<Identity, &'static str> {
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return Err("no_foreground_window");
        }
        let mut window_process = 0;
        let thread = GetWindowThreadProcessId(window, Some(&mut window_process));
        if thread == 0 {
            return Err("window_unavailable");
        }
        let mut gui = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(thread, &mut gui).map_err(|_| "focus_unavailable")?;
        if gui.hwndFocus.0.is_null() {
            return Err("no_keyboard_focus");
        }
        // UIA supplements the native focus identity; it is not a prerequisite
        // for keyboard input. Document, custom editors and terminals are valid.
        let (process, runtime, is_password) = if use_accessibility {
            bounded_accessibility_identity().unwrap_or((window_process as i32, vec![], None))
        } else {
            (window_process as i32, vec![], None)
        };
        if process == -1 {
            return Err("read_only_or_non_input");
        }
        if GetForegroundWindow() != window {
            return Err("focus_changed_during_capture");
        }
        Ok(Identity {
            window: window.0 as isize,
            window_process,
            focus: gui.hwndFocus.0 as isize,
            process,
            runtime,
            is_password,
        })
    }
}
// A broken accessibility provider must not stall an MCP call indefinitely.
// Only this read-only probe runs in the worker: timed-out workers never type.
fn bounded_accessibility_identity() -> Option<(i32, Vec<i32>, Option<bool>)> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("vault-focus-probe".into())
        .spawn(move || {
            let result = Apartment::new()
                .ok()
                .and_then(|_apartment| accessibility_identity());
            BUSY.store(false, Ordering::SeqCst);
            let _ = sender.send(result);
        });
    if spawned.is_err() {
        BUSY.store(false, Ordering::SeqCst);
        return None;
    }
    receiver
        .recv_timeout(Duration::from_millis(1200))
        .ok()
        .flatten()
}
// Never inspect names, text ranges or values. Unsupported accessibility
// providers fall back to Win32 HWND/focus identity instead of blocking typing.
fn accessibility_identity() -> Option<(i32, Vec<i32>, Option<bool>)> {
    unsafe {
        let automation: IUIAutomation2 =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
        automation.SetConnectionTimeout(500).ok()?;
        automation.SetTransactionTimeout(500).ok()?;
        let element = automation.GetFocusedElement().ok()?;
        if !element.CurrentHasKeyboardFocus().ok()?.as_bool() {
            return None;
        }
        if !element.CurrentIsEnabled().ok()?.as_bool()
            || element.CurrentIsOffscreen().ok()?.as_bool()
            || element.CurrentControlType().ok()? == UIA_ButtonControlTypeId
        {
            return Some((-1, vec![], None));
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            && pattern.CurrentIsReadOnly().ok()?.as_bool()
        {
            return Some((-1, vec![], None));
        }
        let array = element.GetRuntimeId().ok()?;
        if array.is_null() {
            return None;
        }
        let runtime = (|| {
            let low = SafeArrayGetLBound(array, 1).ok()?;
            let high = SafeArrayGetUBound(array, 1).ok()?;
            if high < low || high - low > 128 {
                return None;
            }
            let mut data = std::ptr::null_mut();
            SafeArrayAccessData(array, &mut data).ok()?;
            let values =
                std::slice::from_raw_parts(data.cast::<i32>(), (high - low + 1) as usize).to_vec();
            SafeArrayUnaccessData(array).ok()?;
            Some(values)
        })();
        let _ = SafeArrayDestroy(array);
        Some((
            element.CurrentProcessId().ok()?,
            runtime?,
            element.CurrentIsPassword().ok().map(|b| b.as_bool()),
        ))
    }
}
impl Targets {
    pub fn capture(&self, request: CaptureRequest) -> crate::error::Result<FillStatus> {
        request.validate()?;
        if request.field_name.is_some() {
            return Ok(self.remember(locator::capture(request.clone()), request.expected_password));
        }
        // Chromium may enable its accessibility tree on the first UIA query.
        // Capture retries do not fetch any credential or dispatch input.
        let mut captured = identity(true);
        for _ in 0..3 {
            if captured.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            captured = identity(true);
        }
        Ok(self.remember(captured, request.expected_password))
    }
    fn remember(
        &self,
        captured: std::result::Result<Identity, &'static str>,
        expected_password: Option<bool>,
    ) -> FillStatus {
        let identity = match captured {
            Ok(identity) => identity,
            Err(reason) => {
                let mut status = FillStatus::status("unsupported_target");
                status.reason = Some(reason);
                return status;
            }
        };
        if expected_password.is_some() && identity.is_password != expected_password {
            let mut status = FillStatus::status("unsupported_target");
            status.reason = Some("field_type_mismatch");
            status.is_password = identity.is_password;
            return status;
        }
        let Ok(mut items) = self.items.lock() else {
            return FillStatus::status("input_failed");
        };
        items.retain(|_, target| target.created.elapsed() < TTL);
        if items.len() >= 64 {
            return FillStatus::status("input_failed");
        }
        let process_id = identity.window_process;
        let is_password = identity.is_password;
        let id = uuid::Uuid::new_v4().to_string();
        items.insert(
            id.clone(),
            Target {
                identity,
                created: Instant::now(),
            },
        );
        FillStatus {
            status: "target_captured",
            target_id: Some(id),
            expires_in_seconds: Some(TTL.as_secs()),
            process_id: Some(process_id),
            reason: None,
            is_password,
        }
    }
    pub fn fill<F>(&self, id: &str, value: F) -> crate::error::Result<FillStatus>
    where
        F: FnOnce() -> crate::error::Result<String>,
    {
        let target = match self
            .items
            .lock()
            .map_err(|_| crate::error::AppError::storage())?
            .remove(id)
        {
            Some(target) => target,
            None => return Ok(FillStatus::status("target_unavailable")),
        };
        if target.created.elapsed() >= TTL {
            return Ok(FillStatus::status("target_expired"));
        }
        if unsafe { GetForegroundWindow().0 as isize } != target.identity.window {
            return Ok(FillStatus::status("target_changed"));
        }
        let current = match identity(!target.identity.runtime.is_empty()) {
            Ok(current) => current,
            Err(_) => return Ok(FillStatus::status("unsupported_target")),
        };
        if current != target.identity {
            return Ok(FillStatus::status("target_changed"));
        }
        let text = Zeroizing::new(value()?);
        if text.is_empty()
            || text.chars().any(|c| c.is_control())
            || text.encode_utf16().count() > 4096
        {
            return Ok(FillStatus::status("invalid_text"));
        }
        let mut inputs = Vec::with_capacity(text.len() * 2);
        for code in text.encode_utf16() {
            for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: code,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }
        }
        // Recheck after obtaining/decrypting the secret. No clipboard, no
        // select-all, no deletion shortcut and no automatic Enter/submission.
        if identity(!target.identity.runtime.is_empty()).ok().as_ref() != Some(&target.identity) {
            return Ok(FillStatus::status("target_changed"));
        }
        if unsafe {
            [0x10, 0x11, 0x12, 0x5b, 0x5c]
                .iter()
                .any(|key| GetAsyncKeyState(*key) < 0)
        } {
            return Ok(FillStatus::status("modifier_held"));
        }
        let count = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        // Wipe keyboard buffers as well as decrypted source text.
        for input in &mut inputs {
            input.Anonymous.ki.wScan = 0;
        }
        Ok(FillStatus::status(if count as usize == inputs.len() {
            "input_sent"
        } else {
            "input_blocked_or_partial"
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fake() -> Identity {
        Identity {
            window: 1,
            window_process: 2,
            focus: 3,
            process: 4,
            runtime: vec![1, 2],
            is_password: None,
        }
    }
    #[test]
    fn invalid_selectors_are_rejected_before_focus() {
        for value in [
            serde_json::json!({"windowTitle":"Edge"}),
            serde_json::json!({"fieldName":"  "}),
            serde_json::json!({"fieldName":"Password\n"}),
            serde_json::json!({"fieldName":"Password","processId":0}),
            serde_json::json!({"fieldName":"x".repeat(257)}),
        ] {
            let request: CaptureRequest = serde_json::from_value(value).unwrap();
            assert!(Targets::default().capture(request).is_err());
        }
        assert!(
            serde_json::from_value::<CaptureRequest>(serde_json::json!({"selector":"#password"}))
                .is_err()
        );
    }
    #[test]
    fn password_type_mismatch_does_not_issue_a_target() {
        let targets = Targets::default();
        for actual in [None, Some(false)] {
            let mut identity = fake();
            identity.is_password = actual;
            let result = targets.remember(Ok(identity), Some(true));
            assert_eq!(result.reason, Some("field_type_mismatch"));
            assert!(result.target_id.is_none());
            assert!(targets.items.lock().unwrap().is_empty());
        }
        let mut identity = fake();
        identity.is_password = Some(true);
        assert!(
            targets
                .remember(Ok(identity), Some(true))
                .target_id
                .is_some()
        );
    }
    #[test]
    fn invalid_and_expired_targets_never_load_secret() {
        let targets = Targets::default();
        assert_eq!(
            targets
                .fill("unknown", || panic!("must not load"))
                .unwrap()
                .status,
            "target_unavailable"
        );
        targets.items.lock().unwrap().insert(
            "expired".into(),
            Target {
                identity: fake(),
                created: Instant::now() - TTL - Duration::from_secs(1),
            },
        );
        assert_eq!(
            targets
                .fill("expired", || panic!("must not load"))
                .unwrap()
                .status,
            "target_expired"
        );
        assert_eq!(
            targets
                .fill("expired", || panic!("must not load"))
                .unwrap()
                .status,
            "target_unavailable"
        );
    }
}
