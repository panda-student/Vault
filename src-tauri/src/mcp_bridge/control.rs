//! Local update coordination. Contains no credentials, client configuration or commands.
use crate::error::{AppError, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn directory(executable: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(executable).map_err(|_| AppError::storage())?;
    let name = format!(
        "{:x}",
        Sha256::digest(canonical.to_string_lossy().to_lowercase().as_bytes())
    );
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(AppError::storage)?)
            .join("LocalPasswordManager")
            .join("updates")
            .join(name),
    )
}
fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    expires: u64,
}
pub fn active(base: &Path) -> Option<String> {
    let request: Request = serde_json::from_slice(&fs::read(base.join("pause.json")).ok()?).ok()?;
    (request.expires > seconds() && uuid::Uuid::parse_str(&request.token).is_ok())
        .then_some(request.token)
}
pub struct Registration {
    path: PathBuf,
    _lease: File,
}
impl Registration {
    pub fn new(base: &Path) -> Result<Self> {
        let path = base.join("bridges").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&path).map_err(|_| AppError::storage())?;
        let lease = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path.join("lease"))
            .map_err(|_| AppError::storage())?;
        lease
            .try_lock_exclusive()
            .map_err(|_| AppError::storage())?;
        Ok(Self {
            path,
            _lease: lease,
        })
    }
    pub fn acknowledge(&self, token: &str) -> Result<()> {
        fs::write(self.path.join("paused"), token).map_err(|_| AppError::storage())
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._lease);
        let _ = fs::remove_dir_all(&self.path);
    }
}
pub struct Pause {
    base: PathBuf,
    pub token: String,
    cleanup: bool,
}
impl Pause {
    pub fn begin(executable: &Path) -> Result<Self> {
        Self::begin_with_timeout(executable, Duration::from_secs(30))
    }
    fn begin_with_timeout(executable: &Path, timeout: Duration) -> Result<Self> {
        let base = directory(executable)?;
        fs::create_dir_all(&base).map_err(|_| AppError::storage())?;
        // Serialize attempts by different desktop windows, including stale requests.
        let coordination = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(base.join("coordination.lock"))
            .map_err(|_| AppError::storage())?;
        coordination
            .try_lock_exclusive()
            .map_err(|_| AppError::new("UPDATE_IN_PROGRESS", "另一个窗口正在准备更新。"))?;
        if active(&base).is_some() {
            return Err(AppError::new(
                "UPDATE_IN_PROGRESS",
                "程序正在准备更新，请稍后重试。",
            ));
        }
        let token = uuid::Uuid::new_v4().to_string();
        let pause = Self {
            base,
            token,
            cleanup: true,
        };
        let request = Request {
            token: pause.token.clone(),
            expires: seconds() + 300,
        };
        // A bridge acknowledges only a fully parsed token. Installation additionally
        // requires all worker leases, so a partial read can never authorize replacement.
        let _ = fs::remove_file(pause.base.join("pause.json"));
        let mut request_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(pause.base.join("pause.json"))
            .map_err(|_| AppError::storage())?;
        request_file
            .write_all(&serde_json::to_vec(&request).map_err(|_| AppError::storage())?)
            .and_then(|_| request_file.sync_all())
            .map_err(|_| AppError::storage())?;
        drop(request_file);
        let until = std::time::Instant::now() + timeout;
        loop {
            let mut ready = true;
            if let Ok(entries) = fs::read_dir(pause.base.join("bridges")) {
                for entry in entries {
                    let path = entry.map_err(|_| AppError::storage())?.path();
                    let Ok(lease) = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(path.join("lease"))
                    else {
                        continue;
                    };
                    if lease.try_lock_exclusive().is_ok() {
                        continue;
                    } // Exited bridge.
                    if !fs::read_to_string(path.join("paused")).is_ok_and(|s| s == pause.token) {
                        ready = false;
                    }
                }
            }
            if ready {
                return Ok(pause);
            }
            if std::time::Instant::now() >= until {
                return Err(AppError::new(
                    "UPDATE_MCP_BUSY",
                    "MCP 操作尚未完成，本次更新已取消，连接会自动恢复。请稍后重试。",
                ));
            }
            std::thread::sleep(Duration::from_millis(75));
        }
    }
    pub fn adopt(executable: &Path, token: &str) -> Result<Self> {
        let base = directory(executable)?;
        Ok(Self {
            base,
            token: token.to_owned(),
            cleanup: true,
        })
    }
    pub fn handoff(mut self) {
        self.cleanup = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_waits_for_ack_and_drop_resumes_only_its_own_pause() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("vault.exe");
        fs::write(&exe, b"fixture").unwrap();
        let base = directory(&exe).unwrap();
        let registration = Registration::new(&base).unwrap();
        let observer = base.clone();
        let thread = std::thread::spawn(move || {
            loop {
                if let Some(token) = active(&observer) {
                    registration.acknowledge(&token).unwrap();
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            registration
        });
        let pause = Pause::begin_with_timeout(&exe, Duration::from_secs(2)).unwrap();
        let registration = thread.join().unwrap();
        assert_eq!(active(&base), Some(pause.token.clone()));
        let other = Pause::adopt(&exe, &uuid::Uuid::new_v4().to_string()).unwrap();
        drop(other);
        assert!(active(&base).is_some());
        drop(pause);
        assert!(active(&base).is_none());
        drop(registration);
    }
    #[test]
    fn drain_timeout_cancels_update_and_removes_pause_request() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("vault.exe");
        fs::write(&exe, b"fixture").unwrap();
        let base = directory(&exe).unwrap();
        let _registration = Registration::new(&base).unwrap();
        let error = Pause::begin_with_timeout(&exe, Duration::from_millis(100))
            .err()
            .unwrap();
        assert_eq!(error.code, "UPDATE_MCP_BUSY");
        assert!(active(&base).is_none());
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        if !self.cleanup {
            return;
        }
        let matching = fs::read(self.base.join("pause.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok())
            .is_some_and(|r| r.token == self.token);
        if matching {
            let _ = fs::remove_file(self.base.join("pause.json"));
        }
    }
}
