//! Windows capability service: startup integration and native human confirmation.
use super::*;
use std::path::Path;
#[derive(Default)]
pub(super) struct WindowsService {
    clipboard: std::sync::RwLock<Option<std::sync::Arc<dyn ClipboardPort>>>,
}
impl WindowsService {
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
