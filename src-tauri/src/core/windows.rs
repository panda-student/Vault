//! Windows capability service: startup integration and native human confirmation.
use super::*;
use std::path::Path;
mod input;
pub(super) mod installer;
use crate::fill::FillStatus;
pub(super) struct UpdateLaunch {
    pub helper: std::path::PathBuf,
    pub plan: std::path::PathBuf,
}
pub(super) fn wait_for_process(pid: u32) -> Result<()> {
    use ::windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let process = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(process) => process,
        Err(error) if error.code().0 as u32 == 0x80070057 => return Ok(()),
        Err(_) => {
            return Err(AppError::new(
                "UPDATE_PARENT",
                "无法等待旧程序退出，未执行更新。",
            ));
        }
    };
    let result = unsafe { WaitForSingleObject(process, 30000) };
    let _ = unsafe { CloseHandle(process) };
    if result.0 != 0 {
        return Err(AppError::new(
            "UPDATE_PARENT",
            "旧程序未能及时退出，未执行更新。",
        ));
    }
    Ok(())
}
pub(super) fn start_updated(
    executable: &Path,
    directory: &Path,
    health: Option<(&Path, &str)>,
) -> Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new(executable);
    command.args([std::ffi::OsStr::new("--data-dir"), directory.as_os_str()]);
    if let Some((path, token)) = health {
        command.arg("--update-health").arg(path).arg(token);
    }
    command
        .creation_flags(0x08000000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| AppError::new("UPDATE_START", "无法启动新版程序。"))
}
#[derive(Default)]
pub(super) struct WindowsService {
    input: input::Targets,
    clipboard: std::sync::RwLock<Option<std::sync::Arc<dyn ClipboardPort>>>,
}
impl WindowsService {
    pub fn capture_target(&self, request: CaptureRequest) -> Result<FillStatus> {
        self.input.capture(request)
    }
    /// The resolver stays lazy: the native adapter validates the target before
    /// obtaining plaintext, then rechecks focus immediately before typing.
    pub fn fill_target<F>(&self, target: &str, resolve: F) -> Result<FillStatus>
    where
        F: FnOnce() -> Result<String>,
    {
        self.input.fill(target, resolve)
    }
    /// Preserve the destination ACL and metadata; never fall back to truncation.
    pub fn replace_file(&self, destination: &Path, replacement: &Path) -> Result<()> {
        use ::windows::{
            Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
            core::PCWSTR,
        };
        use std::os::windows::ffi::OsStrExt;
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let replacement: Vec<u16> = replacement
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            ReplaceFileW(
                PCWSTR(destination.as_ptr()),
                PCWSTR(replacement.as_ptr()),
                PCWSTR::null(),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        }
        .map_err(|_| {
            AppError::new(
                "FILE_REPLACE_FAILED",
                "Local credential write failed; no file contents are returned.",
            )
        })
    }

    pub fn launch_update(&self, launch: UpdateLaunch) -> Result<()> {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(launch.helper)
            .arg("--apply-update")
            .arg(launch.plan)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| AppError::new("UPDATE_START", "无法启动更新辅助进程，旧程序继续运行。"))?;
        Ok(())
    }
    pub fn bind_clipboard(&self, port: std::sync::Arc<dyn ClipboardPort>) -> Result<()> {
        *self.clipboard.write().map_err(|_| AppError::storage())? = Some(port);
        Ok(())
    }
    pub fn copy_text(&self, text: String) -> Result<()> {
        if text.len() > 65536 {
            return Err(AppError::invalid());
        }
        self.clipboard
            .read()
            .map_err(|_| AppError::storage())?
            .as_ref()
            .ok_or(AppError::new("CLIPBOARD_ERROR", "桌面剪贴板尚未就绪。"))?
            .write_text(text)
    }
    pub fn startup_enabled(&self, executable: &Path, directory: &Path) -> Result<bool> {
        crate::startup::enabled(executable, directory)
    }
    pub fn set_startup(&self, executable: &Path, directory: &Path, enabled: bool) -> Result<bool> {
        crate::startup::set(enabled, executable, directory)?;
        self.startup_enabled(executable, directory)
    }
    pub fn confirm_purge(&self, owner_window: isize, count: usize) -> Result<bool> {
        use ::windows::{
            Win32::{
                Foundation::HWND,
                UI::WindowsAndMessaging::{
                    IDYES, IsWindow, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO, MessageBoxW,
                },
            },
            core::PCWSTR,
        };
        let hwnd = HWND(owner_window as *mut std::ffi::c_void);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(AppError::invalid());
        }
        let text: Vec<u16> =
            format!("将永久删除垃圾桶中的全部 {count} 条凭据。此操作无法恢复。是否继续？")
                .encode_utf16()
                .chain(Some(0))
                .collect();
        let title: Vec<u16> = "确认永久清空垃圾桶".encode_utf16().chain(Some(0)).collect();
        let answer = unsafe {
            MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        };
        Ok(answer == IDYES)
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn native_replace_preserves_destination_and_consumes_staging_file() {
        let folder = tempfile::tempdir().unwrap();
        let destination = folder.path().join("config.txt");
        let staged = folder.path().join("staged.txt");
        std::fs::write(&destination, "before").unwrap();
        std::fs::write(&staged, "after").unwrap();
        WindowsService::default()
            .replace_file(&destination, &staged)
            .unwrap();
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "after");
        assert!(!staged.exists());
    }
    #[test]
    fn native_replace_failure_keeps_staging_and_never_creates_destination() {
        let folder = tempfile::tempdir().unwrap();
        let destination = folder.path().join("missing.txt");
        let staged = folder.path().join("staged.txt");
        std::fs::write(&staged, "after").unwrap();
        assert_eq!(
            WindowsService::default()
                .replace_file(&destination, &staged)
                .unwrap_err()
                .code,
            "FILE_REPLACE_FAILED"
        );
        assert!(!destination.exists());
        assert_eq!(std::fs::read_to_string(&staged).unwrap(), "after");
    }
}
