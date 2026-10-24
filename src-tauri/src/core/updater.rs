//! Signed portable updates. Network metadata never supplies local paths or commands.
use crate::error::{AppError, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const ENDPOINT: &str =
    "https://github.com/panda-student/vault/releases/latest/download/latest.json";
const MAX_PACKAGE: u64 = 64 * 1024 * 1024;
const PUBLIC_KEY: &str = include_str!("../../update-public-key.txt");
// Only debug builds can use an isolated local test server. Shipping builds have
// no endpoint/key override and always use the embedded production public key.
fn test_source() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("VAULT_UPDATE_TEST_SOURCE") {
        if let Ok(url) = reqwest::Url::parse(&value) {
            if url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port().is_some()
                && url.path() == "/latest.json"
                && url.username().is_empty()
                && url.password().is_none()
            {
                return Some(value);
            }
        }
    }
    None
}
fn failure(code: &'static str, message: &'static str) -> AppError {
    AppError::new(code, message)
}
fn network_error() -> AppError {
    failure("UPDATE_NETWORK", "无法连接更新源，请检查网络或代理后重试。")
}
fn invalid_update() -> AppError {
    failure("UPDATE_INVALID", "更新包或签名校验失败，原程序未被替换。")
}
fn busy() -> AppError {
    failure("UPDATE_BUSY", "更新操作正在进行，请稍后重试。")
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Envelope {
    pub payload: String,
    pub signature: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema: u32,
    pub version: String,
    pub platform: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub notes: String,
    pub archive_sha256: String,
    pub archive_size: u64,
}

fn decode(envelope: &Envelope, key: &str, current: &str) -> Result<Manifest> {
    let public: [u8; 32] = STANDARD
        .decode(key.trim())
        .map_err(|_| invalid_update())?
        .try_into()
        .map_err(|_| invalid_update())?;
    let bytes = STANDARD
        .decode(&envelope.payload)
        .map_err(|_| invalid_update())?;
    let signature = Signature::from_slice(
        &STANDARD
            .decode(&envelope.signature)
            .map_err(|_| invalid_update())?,
    )
    .map_err(|_| invalid_update())?;
    VerifyingKey::from_bytes(&public)
        .map_err(|_| invalid_update())?
        .verify_strict(&bytes, &signature)
        .map_err(|_| invalid_update())?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| invalid_update())?;
    let version = semver::Version::parse(&manifest.version).map_err(|_| invalid_update())?;
    let current = semver::Version::parse(current).map_err(|_| invalid_update())?;
    if manifest.schema != 2
        || manifest.platform != "windows-x86_64"
        || version <= current
        || !version.pre.is_empty()
        || manifest.size < 2
        || manifest.size > MAX_PACKAGE
        || manifest.notes.len() > 16000
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid_update());
    }
    if !(2..=MAX_PACKAGE).contains(&manifest.archive_size)
        || (manifest.archive_sha256.len() != 64 || !manifest.archive_sha256.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(invalid_update());
    }
    let expected = format!("https://github.com/panda-student/vault/releases/download/v{0}/vault-{0}.zip", manifest.version);
    if manifest.url != expected { return Err(invalid_update()); }
    Ok(manifest)
}
fn verify_binary(path: &Path, manifest: &Manifest) -> Result<()> {
    verify_file(path, manifest.size, &manifest.sha256, b"MZ")
}
fn verify_file(path: &Path, size: u64, hash: &str, magic: &[u8; 2]) -> Result<()> {
    let mut file = File::open(path).map_err(|_| invalid_update())?;
    if file.metadata().map_err(|_| invalid_update())?.len() != size {
        return Err(invalid_update());
    }
    let mut hasher = Sha256::new();
    let mut prefix = [0; 2];
    file.read_exact(&mut prefix).map_err(|_| invalid_update())?;
    if prefix != *magic {
        return Err(invalid_update());
    }
    hasher.update(prefix);
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| invalid_update())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if format!("{:x}", hasher.finalize()) != hash.to_ascii_lowercase() {
        return Err(invalid_update());
    }
    Ok(())
}

/// Read a single fixed member into a new file; archive paths never become destinations.
fn extract_update_payload(archive: &Path, output: &Path, member: &str, size: u64) -> Result<()> {
    let file = File::open(archive).map_err(|_| invalid_update())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| invalid_update())?;
    let index = zip.file_names().position(|name| name == member).ok_or_else(invalid_update)?;
    let mut entry = zip.by_index(index).map_err(|_| invalid_update())?;
    if entry.size() != size || !entry.is_file() { return Err(invalid_update()); }
    let mut destination = OpenOptions::new().write(true).create_new(true).open(output).map_err(|_| invalid_update())?;
    let count = std::io::copy(&mut entry.by_ref().take(size + 1), &mut destination).map_err(|_| invalid_update())?;
    if count != size { return Err(invalid_update()); }
    destination.sync_all().map_err(|_| AppError::storage())?;
    Ok(())
}

/// Desktop and MCP workers hold a shared lease before opening the vault.
/// Connection bridges stay alive while their workers drain and release leases.
pub struct UpdateRuntime {
    pub(crate) executable: PathBuf,
    lease: Mutex<File>,
    pub(crate) cache: PathBuf,
    pause: Mutex<Option<String>>,
}
impl UpdateRuntime {
    pub fn acquire(executable: &Path) -> Result<Arc<Self>> {
        let (lock, cache) = runtime_paths(executable)?;
        fs::create_dir_all(lock.parent().ok_or_else(invalid_update)?)
            .map_err(|_| AppError::storage())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)
            .map_err(|_| AppError::storage())?;
        FileExt::try_lock_shared(&file).map_err(|_| {
            failure(
                "UPDATE_IN_PROGRESS",
                "程序正在更新，请等待更新完成后再启动。",
            )
        })?;
        Ok(Arc::new(Self {
            executable: executable.to_owned(),
            lease: Mutex::new(file),
            cache,
            pause: Mutex::new(None),
        }))
    }
    fn claim(&self) -> Result<()> {
        let file = self.lease.lock().map_err(|_| busy())?;
        FileExt::unlock(&*file).map_err(|_| AppError::storage())?;
        if FileExt::try_lock_exclusive(&*file).is_err() {
            FileExt::try_lock_shared(&*file).map_err(|_| busy())?;
            return Err(failure(
                "UPDATE_CLIENTS_ACTIVE",
                "仍有其他桌面窗口或旧版 MCP 占用程序。请退出其他窗口；旧版 MCP 首次切换需重连一次，新版 MCP 会自动暂停和恢复，无需退出 AI 客户端。",
            ));
        }
        Ok(())
    }
    fn release_claim(&self) {
        if let Ok(file) = self.lease.lock() {
            let _ = FileExt::unlock(&*file);
            let _ = FileExt::try_lock_shared(&*file);
        }
    }
}
fn runtime_paths(executable: &Path) -> Result<(PathBuf, PathBuf)> {
    let base = crate::mcp_bridge::control::directory(executable)?;
    Ok((base.join("sessions.lock"), base.join("cache")))
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub phase: String,
    pub version: Option<String>,
    pub notes: String,
    pub received: u64,
    pub total: u64,
    pub error: Option<String>,
}
#[derive(Default)]
struct State {
    status: UpdateStatus,
    envelope: Option<Envelope>,
    manifest: Option<Manifest>,
    ready: Option<PathBuf>,
}
pub(super) struct UpdateService {
    state: Mutex<State>,
    runtime: Mutex<Option<Arc<UpdateRuntime>>>,
    cancelled: AtomicBool,
    installing: AtomicBool,
}
impl Default for UpdateService {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                status: UpdateStatus {
                    phase: "idle".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            runtime: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            installing: AtomicBool::new(false),
        }
    }
}
impl UpdateService {
    pub fn bind(&self, runtime: Arc<UpdateRuntime>) -> Result<()> {
        *self.runtime.lock().map_err(|_| busy())? = Some(runtime);
        Ok(())
    }
    fn runtime(&self) -> Result<Arc<UpdateRuntime>> {
        self.runtime
            .lock()
            .map_err(|_| busy())?
            .clone()
            .ok_or_else(|| failure("UPDATE_UNAVAILABLE", "更新服务尚未就绪。"))
    }
    pub fn installing(&self) -> bool {
        self.installing.load(Ordering::SeqCst)
    }
    pub fn status(&self) -> Result<UpdateStatus> {
        Ok(self.state.lock().map_err(|_| busy())?.status.clone())
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    fn begin(&self, phase: &str) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| busy())?;
        if matches!(
            state.status.phase.as_str(),
            "checking" | "downloading" | "installing"
        ) {
            return Err(busy());
        }
        state.status.phase = phase.into();
        state.status.error = None;
        self.cancelled.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn failed<T>(&self, error: AppError) -> Result<T> {
        if let Ok(mut state) = self.state.lock() {
            state.status.phase = "error".into();
            state.status.error = Some(if cfg!(debug_assertions) {
                error.to_string()
            } else {
                error.message.into()
            });
        }
        Err(error)
    }
    fn client() -> Result<reqwest::blocking::Client> {
        reqwest::blocking::Client::builder()
            .https_only(test_source().is_none())
            .user_agent(concat!("Vault/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|_| network_error())
    }
    pub fn check(&self) -> Result<UpdateStatus> {
        self.begin("checking")?;
        let result = (|| {
            let response = Self::client()?
                .get(test_source().as_deref().unwrap_or(ENDPOINT))
                .send()
                .map_err(|_| network_error())?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            let response = response.error_for_status().map_err(|_| network_error())?;
            let mut bytes = Vec::new();
            response
                .take(128 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| network_error())?;
            if bytes.len() > 128 * 1024 {
                return Err(invalid_update());
            }
            let envelope: Envelope =
                serde_json::from_slice(&bytes).map_err(|_| invalid_update())?;
            // Authenticate before comparing versions, including the up-to-date case.
            let manifest = decode(&envelope, PUBLIC_KEY, "0.0.0")?;
            if semver::Version::parse(&manifest.version).map_err(|_| invalid_update())?
                <= semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
            {
                return Ok(None);
            }
            Ok(Some((envelope, manifest)))
        })();
        match result {
            Ok(found) => {
                let mut state = self.state.lock().map_err(|_| busy())?;
                match found {
                    Some((envelope, manifest)) => {
                        let same_ready = state.manifest.as_ref().is_some_and(|m| {
                            m.version == manifest.version && m.sha256 == manifest.sha256
                        }) && state.ready.is_some();
                        state.status = UpdateStatus {
                            phase: if same_ready { "ready" } else { "available" }.into(),
                            version: Some(manifest.version.clone()),
                            notes: manifest.notes.clone(),
                            total: manifest.archive_size,
                            ..Default::default()
                        };
                        state.manifest = Some(manifest);
                        state.envelope = Some(envelope);
                        if !same_ready {
                            state.ready = None;
                        }
                    }
                    None => {
                        state.status = UpdateStatus {
                            phase: "up-to-date".into(),
                            ..Default::default()
                        };
                        state.manifest = None;
                        state.envelope = None;
                        state.ready = None;
                    }
                };
                Ok(state.status.clone())
            }
            Err(error) => self.failed(error),
        }
    }
    pub fn download(&self) -> Result<UpdateStatus> {
        self.begin("downloading")?;
        let result = (|| {
            let runtime = self.runtime()?;
            let envelope = self
                .state
                .lock()
                .map_err(|_| busy())?
                .envelope
                .clone()
                .ok_or_else(invalid_update)?;
            let manifest = decode(&envelope, PUBLIC_KEY, env!("CARGO_PKG_VERSION"))?;
            fs::create_dir_all(&runtime.cache).map_err(|_| AppError::storage())?;
            let partial = runtime.cache.join(format!("{}.part", uuid::Uuid::new_v4()));
            let unpacked = runtime.cache.join(format!("{}.exe", uuid::Uuid::new_v4()));
            let result = (|| {
                let download_size = manifest.archive_size;
                let asset = format!("/vault-{}.zip", manifest.version);
                let url = test_source()
                    .map(|url| url.replace("/latest.json", &asset))
                    .unwrap_or_else(|| manifest.url.clone());
                let mut response = Self::client()?
                    .get(url)
                    .send()
                    .map_err(|_| network_error())?
                    .error_for_status()
                    .map_err(|_| network_error())?;
                if response
                    .content_length()
                    .is_some_and(|size| size != download_size)
                {
                    return Err(invalid_update());
                }
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&partial)
                    .map_err(|_| AppError::storage())?;
                let mut received = 0;
                self.state.lock().map_err(|_| busy())?.status.received = 0;
                let mut buffer = [0; 65536];
                loop {
                    if self.cancelled.load(Ordering::SeqCst) {
                        return Err(failure("UPDATE_CANCELLED", "已取消下载，可以重新下载。"));
                    }
                    let count = response.read(&mut buffer).map_err(|_| network_error())?;
                    if count == 0 {
                        break;
                    }
                    received += count as u64;
                    if received > download_size {
                        return Err(invalid_update());
                    }
                    file.write_all(&buffer[..count])
                        .map_err(|_| AppError::storage())?;
                    self.state.lock().map_err(|_| busy())?.status.received = received;
                }
                file.sync_all().map_err(|error| {
                    AppError::storage().with("update.flush", &error.to_string())
                })?;
                drop(file);
                verify_file(&partial, download_size, &manifest.archive_sha256, b"PK")?;
                extract_update_payload(&partial, &unpacked, &format!("vault-{}/vault.exe", manifest.version), manifest.size)?;
                verify_binary(&unpacked, &manifest)?;
                fs::remove_file(&partial).map_err(|_| AppError::storage())?;
                let ready = unpacked.clone();
                let mut state = self.state.lock().map_err(|_| busy())?;
                if let Some(old) = state.ready.replace(ready) {
                    let _ = fs::remove_file(old);
                }
                state.status.phase = "ready".into();
                Ok(state.status.clone())
            })();
            if result.is_err() {
                let _ = fs::remove_file(partial);
                let _ = fs::remove_file(unpacked);
            }
            result
        })();
        match result {
            Ok(status) => Ok(status),
            Err(error) => self.failed(error),
        }
    }
    pub fn pause_connections(&self) -> Result<crate::mcp_bridge::control::Pause> {
        if self.state.lock().map_err(|_| busy())?.status.phase != "ready" {
            return Err(failure("UPDATE_NOT_READY", "请先完成更新包下载。"));
        }
        crate::mcp_bridge::control::Pause::begin(&self.runtime()?.executable)
    }
    pub fn prepare_install(&self, directory: &Path, pause: crate::mcp_bridge::control::Pause) -> Result<super::windows::UpdateLaunch> {
        let runtime = self.runtime()?;
        let (envelope, ready) = {
            let state = self.state.lock().map_err(|_| busy())?;
            if state.status.phase != "ready" {
                return Err(failure("UPDATE_NOT_READY", "请先完成更新包下载。"));
            }
            (
                state.envelope.clone().ok_or_else(invalid_update)?,
                state.ready.clone().ok_or_else(invalid_update)?,
            )
        };
        let manifest = decode(&envelope, PUBLIC_KEY, env!("CARGO_PKG_VERSION"))?;
        verify_binary(&ready, &manifest)?;
        runtime.claim()?;
        let result = (|| {
            let stage = runtime
                .executable
                .parent()
                .ok_or_else(invalid_update)?
                .join(format!(".vault-update-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&stage).map_err(|_| {
                failure(
                    "UPDATE_DIRECTORY",
                    "程序目录不可写，请将便携版移到可写目录后重试。",
                )
            })?;
            let helper = stage.join("helper.exe");
            fs::copy(&runtime.executable, &helper).map_err(|_| AppError::storage())?;
            fs::copy(ready, stage.join("payload.exe")).map_err(|_| AppError::storage())?;
            let plan = InstallPlan {
                executable: runtime.executable.clone(),
                directory: directory.to_owned(),
                parent: std::process::id(),
                envelope,
                token: uuid::Uuid::new_v4().to_string(),
                pause_token: pause.token.clone(),
            };
            fs::write(
                stage.join("plan.json"),
                serde_json::to_vec(&plan).map_err(|_| invalid_update())?,
            )
            .map_err(|_| AppError::storage())?;
            Ok(super::windows::UpdateLaunch {
                helper,
                plan: stage.join("plan.json"),
            })
        })();
        if result.is_err() {
            runtime.release_claim();
        } else {
            *runtime.pause.lock().map_err(|_| busy())? = Some(pause.token.clone());
            pause.handoff();
        }
        result
    }
    pub fn launched(&self) {
        self.installing.store(true, Ordering::SeqCst);
        if let Ok(mut state) = self.state.lock() {
            state.status.phase = "installing".into();
        }
    }
    pub fn launch_failed(&self) {
        if let Ok(runtime) = self.runtime() {
            runtime.release_claim();
            if let Ok(mut pending) = runtime.pause.lock() {
                if let Some(token) = pending.take() {
                    let _ = crate::mcp_bridge::control::Pause::adopt(&runtime.executable, &token);
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct InstallPlan {
    executable: PathBuf,
    directory: PathBuf,
    parent: u32,
    envelope: Envelope,
    token: String,
    pause_token: String,
}

fn move_file(source: &Path, target: &Path) -> std::io::Result<()> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(17) => copy_move(source, target),
        Err(error) => Err(error),
    }
}
fn copy_move(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let copied = std::io::copy(&mut input, &mut output).and_then(|_| output.sync_all());
    drop(output);
    drop(input);
    if let Err(error) = copied {
        let _ = fs::remove_file(target);
        return Err(error);
    }
    if let Err(error) = fs::remove_file(source) {
        let _ = fs::remove_file(target);
        return Err(error);
    }
    Ok(())
}

/// Runs from a copy of the old executable, before vault/WebView initialization.
pub fn apply_update(plan_path: &Path) -> Result<()> {
    let current = std::env::current_exe().map_err(|_| invalid_update())?;
    let stage = current.parent().ok_or_else(invalid_update)?;
    if current.file_name().and_then(|n| n.to_str()) != Some("helper.exe")
        || plan_path != stage.join("plan.json")
    {
        return Err(invalid_update());
    }
    let plan: InstallPlan =
        serde_json::from_slice(&fs::read(plan_path).map_err(|_| invalid_update())?)
            .map_err(|_| invalid_update())?;
    if !plan.executable.is_absolute()
        || !plan.directory.is_absolute()
        || plan.executable.parent() != stage.parent()
        || !stage
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".vault-update-"))
        || uuid::Uuid::parse_str(&plan.token).is_err()
    {
        return Err(invalid_update());
    }
    let _resume = crate::mcp_bridge::control::Pause::adopt(&plan.executable, &plan.pause_token)?;
    let manifest = decode(&plan.envelope, PUBLIC_KEY, env!("CARGO_PKG_VERSION"))?;
    let payload = stage.join("payload.exe");
    verify_binary(&payload, &manifest)?;
    super::windows::wait_for_process(plan.parent)?;
    let (lock_path, _) = runtime_paths(&plan.executable)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| AppError::storage())?;
    FileExt::try_lock_exclusive(&lock)
        .map_err(|_| failure("UPDATE_CLIENTS_ACTIVE", "仍有进程使用程序，更新未执行。"))?;
    let backup = stage.join("previous.exe");
    // The old image can remain locked briefly after process exit.
    let mut moved = false;
    for _ in 0..50 {
        if move_file(&plan.executable, &backup).is_ok() {
            moved = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !moved {
        return Err(failure("UPDATE_REPLACE", "无法替换程序，旧版本已保留。"));
    }
    if move_file(&payload, &plan.executable).is_err() {
        let _ = move_file(&backup, &plan.executable);
        return Err(invalid_update());
    }
    FileExt::unlock(&lock).map_err(|_| AppError::storage())?;
    let health = stage.join("healthy");
    let mut child = match super::windows::start_updated(
        &plan.executable,
        &plan.directory,
        Some((&health, &plan.token)),
    ) {
        Ok(child) => Some(child),
        Err(_) => None,
    };
    let mut healthy = false;
    if let Some(process) = child.as_mut() {
        for _ in 0..600 {
            if fs::read_to_string(&health).is_ok_and(|s| s == plan.token) {
                healthy = true;
                break;
            }
            if process.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if healthy {
        // Keep previous.exe as an executable-only recovery copy. Never touch data.
        let _ = fs::remove_file(plan_path);
        let _ = fs::remove_file(health);
        return Ok(());
    }
    if let Some(mut process) = child {
        let _ = process.kill();
        let _ = process.wait();
    }
    FileExt::try_lock_exclusive(&lock).map_err(|_| {
        failure(
            "UPDATE_ROLLBACK",
            "新版未确认启动，仍有进程占用；请从更新暂存目录恢复 previous.exe。",
        )
    })?;
    if let Err(error) = move_file(&plan.executable, &stage.join("failed.exe")) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(invalid_update());
        }
    }
    move_file(&backup, &plan.executable).map_err(|_| invalid_update())?;
    FileExt::unlock(&lock).map_err(|_| AppError::storage())?;
    super::windows::start_updated(&plan.executable, &plan.directory, None)?;
    Err(failure(
        "UPDATE_ROLLED_BACK",
        "新版未能正常启动，已恢复旧程序。数据目录保持原样。",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn zip_reader_rejects_noncanonical_member_name() {
        use std::io::Write;
        let dir=tempfile::tempdir().unwrap();
        let archive=dir.path().join("noncanonical.zip");
        let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("vault-0.5.5/Vault.exe",zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"MZtest").unwrap();zip.finish().unwrap();
        let output=dir.path().join("payload.exe");
        assert!(extract_update_payload(&archive,&output,"vault-0.5.5/vault.exe",6).is_err());
        assert!(!output.exists());
    }
    #[test]
    fn redirected_volume_fallback_preserves_bytes_and_does_not_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("old.exe");
        let target = directory.path().join("new.exe");
        fs::write(&source, b"MZtest").unwrap();
        fs::write(&target, b"existing").unwrap();
        assert!(copy_move(&source, &target).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"MZtest");
        fs::remove_file(&target).unwrap();
        copy_move(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(&target).unwrap(), b"MZtest");
    }
    fn signed(version: &str, bytes: &[u8]) -> (Envelope, String) {
        let key = SigningKey::from_bytes(&[27; 32]);
        let payload = serde_json::to_vec(&Manifest { schema: 2, version: version.into(), platform: "windows-x86_64".into(), url: format!("https://github.com/panda-student/vault/releases/download/v{version}/vault-{version}.zip"), sha256: format!("{:x}", Sha256::digest(bytes)), size: bytes.len() as u64, notes: "test".into(), archive_sha256: "a".repeat(64), archive_size: 100 }).unwrap();
        (
            Envelope {
                payload: STANDARD.encode(&payload),
                signature: STANDARD.encode(key.sign(&payload).to_bytes()),
            },
            STANDARD.encode(key.verifying_key().to_bytes()),
        )
    }
    #[test]
    fn signed_payload_rejects_tampering_wrong_key_and_downgrade() {
        let (mut envelope, key) = signed("1.2.3", b"MZtest");
        assert!(decode(&envelope, &key, "1.2.2").is_ok());
        assert!(decode(&envelope, &key, "1.2.3").is_err());
        assert!(decode(&envelope, &STANDARD.encode([9; 32]), "1.0.0").is_err());
        envelope.payload = STANDARD.encode(b"tampered");
        assert!(decode(&envelope, &key, "1.0.0").is_err());
    }
    #[test]
    fn updates_reject_previous_repositories_and_manifest_formats() {
        let (mut envelope, key) = signed("1.2.3", b"MZtest");
        let signing_key = SigningKey::from_bytes(&[27; 32]);
        let mut manifest = decode(&envelope, &key, "1.0.0").unwrap();
        for (repository, accepted) in [("local-password-manager", false), ("untrusted", false)] {
            manifest.url = format!("https://github.com/panda-student/{repository}/releases/download/v1.2.3/vault.exe");
            let payload = serde_json::to_vec(&manifest).unwrap();
            envelope.payload = STANDARD.encode(&payload);
            envelope.signature = STANDARD.encode(signing_key.sign(&payload).to_bytes());
            assert_eq!(decode(&envelope, &key, "1.0.0").is_ok(), accepted);
        }
        manifest.schema = 1;
        let payload = serde_json::to_vec(&manifest).unwrap();
        envelope.payload = STANDARD.encode(&payload);
        envelope.signature = STANDARD.encode(signing_key.sign(&payload).to_bytes());
        assert!(decode(&envelope, &key, "1.0.0").is_err());
    }
    #[test]
    fn signed_zip_rejects_incomplete_metadata_and_wrong_asset() {
        let (mut envelope, key) = signed("1.2.3", b"MZtest");
        let signing_key = SigningKey::from_bytes(&[27; 32]);
        let manifest = decode(&envelope, &key, "1.0.0").unwrap();
        let mut invalid = serde_json::to_value(&manifest).unwrap();
        let variants = [
            { let mut v = invalid.clone(); v.as_object_mut().unwrap().remove("archive_size"); v },
            { let mut v = invalid.clone(); v["archive_sha256"] = serde_json::json!("x".repeat(64)); v },
            { let mut v = invalid.clone(); v["archive_size"] = serde_json::json!(0); v },
            { invalid["url"] = serde_json::json!("https://github.com/panda-student/vault/releases/download/v1.2.3/vault.exe"); invalid },
        ];
        for value in variants {
            let payload = serde_json::to_vec(&value).unwrap();
            envelope.payload = STANDARD.encode(&payload);
            envelope.signature = STANDARD.encode(signing_key.sign(&payload).to_bytes());
            assert!(decode(&envelope, &key, "1.0.0").is_err());
        }
    }
    #[test]
    fn zip_extraction_only_writes_expected_member_and_enforces_size() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive.zip");
        let output = dir.path().join("payload.exe");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        for name in ["vault-1.2.3/vault.exe", "../outside.txt"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"MZtest").unwrap();
        }
        zip.finish().unwrap();
        assert!(extract_update_payload(&archive, &output, "vault-1.2.3/vault.exe", 6).is_ok());
        assert_eq!(fs::read(&output).unwrap(), b"MZtest");
        assert!(!dir.path().parent().unwrap().join("outside.txt").exists());
        assert!(extract_update_payload(&archive, &output, "vault-1.2.3/vault.exe", 6).is_err());
        assert!(extract_update_payload(&archive, &dir.path().join("bad.exe"), "vault-1.2.3/vault.exe", 7).is_err());
    }
    #[test]
    fn signature_binds_binary_hash_size_and_stable_channel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.exe");
        fs::write(&path, b"MZtest").unwrap();
        let (envelope, key) = signed("1.2.3", b"MZtest");
        let manifest = decode(&envelope, &key, "1.0.0").unwrap();
        assert!(verify_binary(&path, &manifest).is_ok());
        fs::write(&path, b"MZevil").unwrap();
        assert!(verify_binary(&path, &manifest).is_err());
        let (preview, key) = signed("1.2.4-beta.1", b"MZtest");
        assert!(decode(&preview, &key, "1.0.0").is_err());
    }
    #[test]
    fn other_sessions_prevent_replacement_and_recover_after_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.exe");
        fs::write(&path, b"test").unwrap();
        let one = UpdateRuntime::acquire(&path).unwrap();
        let two = UpdateRuntime::acquire(&path).unwrap();
        assert_eq!(one.claim().unwrap_err().code, "UPDATE_CLIENTS_ACTIVE");
        drop(two);
        one.claim().unwrap();
        assert!(UpdateRuntime::acquire(&path).is_err());
        one.release_claim();
        assert!(UpdateRuntime::acquire(&path).is_ok());
    }
}
