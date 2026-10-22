//! Package replacement bootstrap. No vault is opened and no credential is read.
use super::windows::{self, installer as platform};
use crate::{
    error::{AppError, Result},
    mcp_bridge::control::Pause,
};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
    process::Child,
    time::{Duration, Instant},
};
fn error(code: &'static str, message: &'static str) -> AppError {
    AppError::new(code, message)
}
fn storage() -> AppError {
    AppError::storage()
}
fn hash(path: &Path) -> Result<Vec<u8>> {
    let mut file = File::open(path).map_err(|_| storage())?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| storage())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest.finalize().to_vec())
}
fn exclusive(lock: &File) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if lock.try_lock_exclusive().is_ok() {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(error(
                "INSTALLER_BUSY",
                "仍有旧版进程使用程序；安装未执行，连接会恢复。",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn replace_package(target: &Path, report: &Path) -> Result<()> {
    if !target.is_absolute()
        || target.file_name().and_then(|s| s.to_str()) != Some("vault.exe")
        || !report.is_absolute()
    {
        return Err(AppError::invalid());
    }
    let current = std::env::current_exe().map_err(|_| storage())?;
    let parent = fs::canonicalize(target.parent().ok_or_else(storage)?).map_err(|_| storage())?;
    let target = parent.join("vault.exe");
    if fs::canonicalize(&current).ok() == fs::canonicalize(&target).ok() {
        return Err(AppError::invalid());
    }
    let install_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(".vault-installer.lock"))
        .map_err(|_| storage())?;
    install_lock
        .try_lock_exclusive()
        .map_err(|_| error("INSTALLER_BUSY", "另一个安装程序正在更新此目录。"))?;
    let existing = target.exists();
    // The retained stdio bridges acknowledge only after all in-flight calls finish.
    let _pause = if existing {
        Some(Pause::begin(&target)?)
    } else {
        None
    };
    let desktops = if existing {
        platform::desktops(&target)?
    } else {
        Vec::new()
    };
    let stage = parent.join(format!(".vault-update-install-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage).map_err(|_| storage())?;
    let backup = stage.join("previous.exe");
    let payload = stage.join("payload.exe");
    fs::copy(&current, &payload).map_err(|_| storage())?;
    if hash(&current)? != hash(&payload)? {
        return Err(storage());
    }
    let session_lock = if existing {
        let base = crate::mcp_bridge::control::directory(&target)?;
        Some(
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(base.join("sessions.lock"))
                .map_err(|_| storage())?,
        )
    } else {
        None
    };
    let mut restarted: Vec<Child> = Vec::new();
    let mut replaced = false;
    let mut backed_up = false;
    let result = (|| {
        platform::close(&desktops)?;
        if let Some(lock) = &session_lock {
            exclusive(lock)?;
        }
        if existing {
            // Rename instead of overwrite: retained stdio bridges may still map the old image.
            fs::rename(&target, &backup)
                .map_err(|_| error("INSTALLER_REPLACE", "无法移动旧程序；旧版本已保留。"))?;
            backed_up = true;
        }
        fs::rename(&payload, &target).map_err(|_| storage())?;
        replaced = true;
        if let Some(lock) = &session_lock {
            FileExt::unlock(lock).map_err(|_| storage())?;
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        for desktop in &desktops {
            let token = uuid::Uuid::new_v4().to_string();
            let healthy = stage.join("healthy");
            let _ = fs::remove_file(&healthy);
            restarted.push(windows::start_updated(
                &target,
                &desktop.directory,
                Some((&healthy, &token)),
            )?);
            loop {
                if fs::read_to_string(&healthy).is_ok_and(|s| s == token) {
                    break;
                }
                if restarted
                    .last_mut()
                    .unwrap()
                    .try_wait()
                    .map_err(|_| storage())?
                    .is_some()
                    || Instant::now() >= deadline
                {
                    return Err(error("INSTALLER_HEALTH", "新版未确认启动，将恢复旧程序。"));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let result = serde_json::json!({"status":"installed","restartedDesktops":restarted.len(),"previousExecutable":if existing{Some(&backup)}else{None}});
        fs::write(report, serde_json::to_vec(&result).map_err(|_| storage())?)
            .map_err(|_| storage())?;
        if !restarted.is_empty() {
            fs::write(report.with_extension("restarted"), b"ready").map_err(|_| storage())?;
        }
        Ok(())
    })();
    if let Err(failure) = result {
        // Only terminate unconfirmed children started by this helper, never AI clients.
        for child in &mut restarted {
            let _ = child.kill();
            let _ = child.wait();
        }
        if backed_up {
            if let Some(lock) = &session_lock {
                exclusive(lock)?;
            }
            if replaced {
                fs::rename(&target, stage.join("failed.exe")).map_err(|_| {
                    error("INSTALLER_ROLLBACK", "恢复旧程序失败；请保留安装暂存目录。")
                })?;
            }
            fs::rename(&backup, &target)
                .map_err(|_| error("INSTALLER_ROLLBACK", "恢复旧程序失败；请保留安装暂存目录。"))?;
        }
        if let Some(lock) = &session_lock {
            let _ = FileExt::unlock(lock);
        }
        for desktop in &desktops {
            if desktop.exited() {
                let _ = windows::start_updated(&target, &desktop.directory, None);
            }
        }
        let _ = fs::write(
            report,
            serde_json::to_vec(&serde_json::json!({"status":"rolled_back","code":failure.code}))
                .unwrap(),
        );
        return Err(failure);
    }
    // Pause drops last, so clients reconnect only after restart or complete rollback.
    Ok(())
}
