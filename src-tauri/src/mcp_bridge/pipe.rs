//! Same-user named pipes. No TCP listener, HTTP endpoint or external runtime.
use sha2::{Digest, Sha256};
use std::{
    io,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER, TokenUser,
        },
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
pub(super) fn process_sid(process: HANDLE) -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).map_err(io::Error::other)?;
        let token = OwnedHandle(token);
        let mut size = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut size);
        if size == 0 {
            return Err(io::Error::other("Token information unavailable"));
        }
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )
        .map_err(io::Error::other)?;
        let info = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(info.User.Sid, &mut text).map_err(io::Error::other)?;
        let sid = text.to_string().map_err(io::Error::other);
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}
pub fn current_sid() -> io::Result<String> {
    process_sid(unsafe { GetCurrentProcess() })
}
pub fn identity(base: &Path, directory: &Path) -> io::Result<(String, PathBuf)> {
    std::fs::create_dir_all(directory)?;
    let data = std::fs::canonicalize(directory)?;
    let key = format!(
        "{}|{}|{}",
        base.to_string_lossy().to_lowercase(),
        data.to_string_lossy().to_lowercase(),
        current_sid()?
    );
    let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    Ok((
        format!(r"\\.\pipe\vault-{hash}"),
        base.join("services").join(hash),
    ))
}
pub fn create(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    // The default pipe ACL includes Everyone/anonymous read access. Set an explicit
    // protected DACL instead, granting only this Windows identity and LocalSystem.
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", current_sid()?)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )
        .map_err(io::Error::other)?;
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            );
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        result
    }
}
pub fn verify_server(pipe: &NamedPipeClient) -> io::Result<u32> {
    let mut pid = 0;
    unsafe {
        GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid)
            .map_err(io::Error::other)?;
        let process = OwnedHandle(
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(io::Error::other)?,
        );
        if process_sid(process.0)? != current_sid()? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Unexpected pipe owner",
            ));
        }
    }
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipe_identity_is_stable_per_vault_and_separates_different_vaults() {
        let dir = tempfile::tempdir().unwrap();
        let a = identity(dir.path(), &dir.path().join("one")).unwrap();
        assert_eq!(a, identity(dir.path(), &dir.path().join("one")).unwrap());
        assert_ne!(a, identity(dir.path(), &dir.path().join("two")).unwrap());
    }
}
