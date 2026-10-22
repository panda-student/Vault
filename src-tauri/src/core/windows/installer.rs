//! Installer OS adapter. Scope shutdown to same-user desktop processes at one path.
use crate::error::{AppError, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, HWND, LPARAM, LocalFree},
        System::{
            Com::*,
            RestartManager::*,
            Threading::*,
            Variant::{VARIANT, VariantClear, VariantToString},
            Wmi::*,
        },
        UI::{
            Shell::CommandLineToArgvW,
            WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId},
        },
    },
    core::{BOOL, BSTR, PCWSTR, PWSTR, w},
};
fn failed() -> AppError {
    AppError::new(
        "INSTALLER_DESKTOP",
        "无法安全退出旧版 Vault；安装已停止，MCP 会恢复。",
    )
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
pub(in crate::core) struct Desktop {
    process: Handle,
    unique: RM_UNIQUE_PROCESS,
    pub directory: PathBuf,
}
impl Desktop {
    pub fn exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.process.0, 0).0 == 0 }
    }
}
unsafe extern "system" fn enumerate(window: HWND, value: LPARAM) -> BOOL {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
        (&mut *(value.0 as *mut BTreeSet<u32>)).insert(pid);
    }
    true.into()
}
fn command_line(pid: u32) -> Result<Vec<String>> {
    unsafe {
        let locator: IWbemLocator =
            CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(|_| failed())?;
        let empty = BSTR::default();
        let services = locator
            .ConnectServer(
                &BSTR::from("ROOT\\CIMV2"),
                &empty,
                &empty,
                &empty,
                0,
                &empty,
                None,
            )
            .map_err(|_| failed())?;
        CoSetProxyBlanket(
            &services,
            10,
            0,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
        .map_err(|_| failed())?;
        let query = services
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(format!(
                    "SELECT CommandLine FROM Win32_Process WHERE ProcessId={pid}"
                )),
                WBEM_FLAG_RETURN_IMMEDIATELY | WBEM_FLAG_FORWARD_ONLY,
                None,
            )
            .map_err(|_| failed())?;
        let mut objects = [None];
        let mut count = 0;
        query
            .Next(5000, &mut objects, &mut count)
            .ok()
            .map_err(|_| failed())?;
        let object = objects[0].as_ref().ok_or_else(failed)?;
        let mut value = VARIANT::default();
        object
            .Get(w!("CommandLine"), 0, &mut value, None, None)
            .map_err(|_| failed())?;
        let mut buffer = vec![0u16; 32768];
        let converted = VariantToString(&value, &mut buffer);
        let _ = VariantClear(&mut value);
        converted.map_err(|_| failed())?;
        let mut argc = 0;
        let argv = CommandLineToArgvW(PCWSTR(buffer.as_ptr()), &mut argc);
        if argv.is_null() {
            return Err(failed());
        }
        let result = std::slice::from_raw_parts(argv, argc as usize)
            .iter()
            .map(|arg| arg.to_string())
            .collect::<std::result::Result<Vec<_>, _>>();
        let _ = LocalFree(Some(HLOCAL(argv.cast())));
        result.map_err(|_| failed())
    }
}
pub(in crate::core) fn desktops(executable: &Path) -> Result<Vec<Desktop>> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|_| failed())?;
        let _apartment = Apartment;
        // Default process COM security, with per-proxy impersonation for local WMI.
        let security = CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        );
        if let Err(error) = security {
            if error.code().0 as u32 != 0x80010119 {
                return Err(failed());
            }
        }
        let mut pids = BTreeSet::new();
        EnumWindows(
            Some(enumerate),
            LPARAM(&mut pids as *mut BTreeSet<u32> as isize),
        )
        .map_err(|_| failed())?;
        let target = std::fs::canonicalize(executable).map_err(|_| failed())?;
        let mut result = Vec::new();
        for pid in pids {
            let Ok(handle) = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            ) else {
                continue;
            };
            let handle = Handle(handle);
            let mut path = vec![0u16; 32768];
            let mut len = path.len() as u32;
            if QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut len,
            )
            .is_err()
            {
                continue;
            }
            let image = PathBuf::from(String::from_utf16_lossy(&path[..len as usize]));
            if std::fs::canonicalize(image).ok().as_ref() != Some(&target) {
                continue;
            }
            if !crate::mcp_bridge::same_user_process(handle.0).map_err(|_| failed())? {
                return Err(failed());
            }
            let args = command_line(pid)?;
            if args.iter().any(|a| a == "--mcp" || a == "--mcp-service") {
                continue;
            }
            let directory = desktop_directory(&args, &target)?;
            let mut created = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(handle.0, &mut created, &mut exit, &mut kernel, &mut user)
                .map_err(|_| failed())?;
            result.push(Desktop {
                process: handle,
                unique: RM_UNIQUE_PROCESS {
                    dwProcessId: pid,
                    ProcessStartTime: created,
                },
                directory,
            });
        }
        if result.len() > 16 {
            return Err(failed());
        }
        Ok(result)
    }
}
fn desktop_directory(args: &[String], executable: &Path) -> Result<PathBuf> {
    let mut directory = executable.parent().ok_or_else(failed)?.join("data");
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => directory = PathBuf::from(args.next().ok_or_else(failed)?),
            "--background" => {}
            "--update-health" => {
                let marker = PathBuf::from(args.next().ok_or_else(failed)?);
                let token = args.next().ok_or_else(failed)?;
                if !marker.is_absolute()
                    || marker.file_name().and_then(|n| n.to_str()) != Some("healthy")
                    || uuid::Uuid::parse_str(token).is_err()
                {
                    return Err(failed());
                }
            }
            // Do not stop unknown modes or silently lose their launch context.
            _ => return Err(failed()),
        }
    }
    if !directory.is_absolute() {
        return Err(failed());
    }
    Ok(directory)
}
pub(in crate::core) fn close(desktops: &[Desktop]) -> Result<()> {
    if desktops.is_empty() {
        return Ok(());
    }
    struct Session(u32);
    impl Drop for Session {
        fn drop(&mut self) {
            let _ = unsafe { RmEndSession(self.0) };
        }
    }
    unsafe {
        let mut id = 0;
        let mut key = [0u16; 33];
        RmStartSession(&mut id, None, PWSTR(key.as_mut_ptr()))
            .ok()
            .map_err(|_| failed())?;
        let _session = Session(id);
        let applications: Vec<_> = desktops.iter().map(|d| d.unique).collect();
        RmRegisterResources(id, None, Some(&applications), None)
            .ok()
            .map_err(|_| failed())?;
        // No force flag: applications may refuse shutdown. Never kill the AI host.
        RmShutdown(id, 0, None).ok().map_err(|_| failed())?;
        for desktop in desktops {
            if WaitForSingleObject(desktop.process.0, 30000).0 != 0 {
                return Err(failed());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_preserves_custom_directory_and_rejects_unknown_modes() {
        let exe = Path::new(r"D:\vault\vault.exe");
        let args = vec![
            exe.to_string_lossy().into_owned(),
            "--background".into(),
            "--data-dir".into(),
            r"D:\Private data\vault".into(),
        ];
        assert_eq!(
            desktop_directory(&args, exe).unwrap(),
            PathBuf::from(r"D:\Private data\vault")
        );
        let mut with_health = args.clone();
        with_health.extend([
            "--update-health".into(),
            r"D:\vault\.vault-update-install-test\healthy".into(),
            uuid::Uuid::new_v4().to_string(),
        ]);
        assert_eq!(
            desktop_directory(&with_health, exe).unwrap(),
            PathBuf::from(r"D:\Private data\vault")
        );
        assert!(desktop_directory(&["vault.exe".into(), "--mcp".into()], exe).is_err());
        assert!(
            desktop_directory(
                &["vault.exe".into(), "--data-dir".into(), "relative".into()],
                exe
            )
            .is_err()
        );
    }
}
