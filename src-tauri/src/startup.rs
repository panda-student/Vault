use crate::error::{AppError, Result};
use std::path::Path;
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
};
const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const NAME: &str = "LocalPasswordManager.Portable";
fn command(executable: &Path, data_directory: &Path) -> String {
    format!(
        "\"{}\" --data-dir \"{}\" --background",
        executable.display(),
        data_directory.display()
    )
}
pub fn enabled(executable: &Path, data_directory: &Path) -> Result<bool> {
    enabled_at(KEY, NAME, &command(executable, data_directory))
}
pub fn set(enabled: bool, executable: &Path, data_directory: &Path) -> Result<()> {
    set_at(KEY, NAME, enabled, &command(executable, data_directory))
}
fn enabled_at(path: &str, name: &str, expected: &str) -> Result<bool> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    match root.open_subkey_with_flags(path, KEY_READ) {
        Ok(key) => match key.get_value::<String, _>(name) {
            Ok(value) => Ok(value == expected),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(AppError::new(
                "STARTUP_ERROR",
                "无法读取当前用户的启动设置。",
            )),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(AppError::new(
            "STARTUP_ERROR",
            "无法读取当前用户的启动设置。",
        )),
    }
}
fn set_at(path: &str, name: &str, enabled: bool, value: &str) -> Result<()> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    if enabled {
        let (key, _) = root
            .create_subkey(path)
            .map_err(|_| AppError::new("STARTUP_ERROR", "无法开启登录后启动。"))?;
        key.set_value(name, &value)
            .map_err(|_| AppError::new("STARTUP_ERROR", "无法开启登录后启动。"))?;
    } else {
        match root.open_subkey_with_flags(path, KEY_WRITE) {
            Ok(key) => match key.delete_value(name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(AppError::new("STARTUP_ERROR", "无法关闭登录后启动。")),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(AppError::new("STARTUP_ERROR", "无法关闭登录后启动。")),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_registry_roundtrip_and_quoted_paths() {
        let path = format!(
            "Software\\LocalPasswordManager\\Acceptance\\{}",
            uuid::Uuid::new_v4()
        );
        let value = command(
            Path::new("D:\\中文 路径\\App.exe"),
            Path::new("D:\\中文 数据"),
        );
        assert!(value.starts_with('"'));
        assert!(value.contains("\" --data-dir \""));
        assert!(value.ends_with("--background"));
        assert!(!enabled_at(&path, NAME, &value).unwrap());
        set_at(&path, NAME, true, &value).unwrap();
        assert!(enabled_at(&path, NAME, &value).unwrap());
        set_at(&path, NAME, false, &value).unwrap();
        assert!(!enabled_at(&path, NAME, &value).unwrap());
        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey(&path)
            .unwrap();
    }
}
