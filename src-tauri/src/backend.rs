//! One Rust backend host for desktop and MCP, per user/executable/vault.
//! The desktop activation endpoint carries no core operations or caller identity.
use crate::mcp_bridge::{control, pipe};
use crate::{
    core::CoreSession,
    error::{AppError, Result},
};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::windows::named_pipe::ClientOptions,
};

pub struct Host {
    _lease: File,
    name: String,
    base: PathBuf,
    executable: PathBuf,
    active: Arc<AtomicUsize>,
}
impl Host {
    /// Acquire ownership before opening any core or database. Losing launchers
    /// activate the existing host; they never open another password vault.
    pub fn acquire(executable: &Path, directory: &Path) -> Result<Option<Self>> {
        let base = control::directory(executable)?;
        let (name, state) = pipe::identity(&base, directory).map_err(|_| AppError::storage())?;
        fs::create_dir_all(&state).map_err(|_| AppError::storage())?;
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(state.join("service.lock"))
            .map_err(|_| AppError::storage())?;
        if lease.try_lock_exclusive().is_err() {
            return Ok(None);
        }
        Ok(Some(Self {
            _lease: lease,
            name,
            base,
            executable: executable.to_owned(),
            active: Arc::new(AtomicUsize::new(0)),
        }))
    }
    pub fn has_clients(&self) -> bool { self.active.load(Ordering::SeqCst) > 0 }
    pub fn start(&self, app: tauri::AppHandle, core: Arc<CoreSession>) -> Result<()> {
        let active = self.active.clone();
        let name = self.name.clone();
        let base = self.base.clone();
        let executable = self.executable.clone();
        let (ready, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async {
                    let listener = pipe::create(&name, true)?;
                    let activation = pipe::create(&format!("{name}-desktop"), true)?;
                    let registration =
                        control::Registration::new(&base).map_err(std::io::Error::other)?;
                    let _ = ready.send(Ok(()));
                    serve(
                        app,
                        core,
                        name,
                        executable,
                        base,
                        registration,
                        listener,
                        activation,
                        active,
                    )
                    .await
                })
            })();
            if let Err(error) = result {
                let _ = ready.send(Err(error.to_string()));
            }
        });
        match receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(()),
            _ => Err(AppError::new("BACKEND_START", "无法启动本地后端连接服务。")),
        }
    }
}

pub fn activate(
    executable: &Path,
    directory: &Path,
    background: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let base = control::directory(executable)?;
    let (name, _) = pipe::identity(&base, directory)?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let until = Instant::now() + Duration::from_secs(15);
            loop {
                if let Ok(mut connection) = ClientOptions::new().open(&format!("{name}-desktop")) {
                    pipe::verify_server(&connection)?;
                    connection
                        .write_all(if background {
                            b"background\n"
                        } else {
                            b"show\n"
                        })
                        .await?;
                    let mut line = String::new();
                    tokio::time::timeout(
                        Duration::from_secs(10),
                        BufReader::new(connection).read_line(&mut line),
                    )
                    .await??;
                    if line.trim() == "ready" {
                        return Ok::<_, std::io::Error>(());
                    }
                    return Err(std::io::Error::other(
                        "Existing backend could not open desktop",
                    ));
                }
                if Instant::now() >= until {
                    return Err(std::io::Error::other("Existing backend is not ready"));
                }
                tokio::time::sleep(Duration::from_millis(75)).await;
            }
        })?;
    Ok(())
}

async fn serve(
    app: tauri::AppHandle,
    core: Arc<CoreSession>,
    name: String,
    executable: PathBuf,
    base: PathBuf,
    registration: control::Registration,
    mut listener: tokio::net::windows::named_pipe::NamedPipeServer,
    mut activation: tokio::net::windows::named_pipe::NamedPipeServer,
    active: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    let mcp = core.mcp_session();
    let mut idle_since = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(75));
    loop {
        tokio::select! {
            connected = listener.connect(), if control::active(&base).is_none() => {
                connected?;
                let next = pipe::create(&name, false)?;
                let connection = std::mem::replace(&mut listener, next);
                let core = mcp.clone();let active = active.clone();
                active.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    struct ClientCount(Arc<AtomicUsize>);
                    impl Drop for ClientCount { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
                    let _count = ClientCount(active);
                    let (read, write) = tokio::io::split(connection);
                    let _ = crate::mcp::serve_connection(core, read, write).await;
                });
            }
            connected = activation.connect() => {
                connected?;
                let next = pipe::create(&format!("{name}-desktop"), false)?;
                let connection = std::mem::replace(&mut activation, next);
                let app = app.clone();let executable = executable.clone();
                let active = active.clone(); active.fetch_add(1, Ordering::SeqCst);
                idle_since = Instant::now();
                tokio::spawn(async move {
                    struct ActivationCount(Arc<AtomicUsize>);
                    impl Drop for ActivationCount { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
                    let _count = ActivationCount(active);
                    if pipe::verify_desktop_client(&connection, &executable).is_err() { return; }
                    let (read, mut write) = tokio::io::split(connection);
                    let mut line = String::new();
                    use tokio::io::AsyncReadExt;
                    if !matches!(tokio::time::timeout(Duration::from_secs(3), BufReader::new(read.take(32)).read_line(&mut line)).await, Ok(Ok(1..))) { return; }
                    let visible = match line.trim() { "show" => true, "background" => false, _ => return };
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    let ui = app.clone();
                    if app.run_on_main_thread(move || { let _ = sender.send(crate::open_main(&ui, visible).is_ok()); }).is_err() { return; }
                    if matches!(receiver.await, Ok(true)) { let _ = write.write_all(b"ready\n").await; }
                });
            }
            _ = tick.tick() => {
                let paused = control::active(&base);
                let count = active.load(Ordering::SeqCst);
                if count > 0 { idle_since = Instant::now(); }
                if count == 0 {
                    if let Some(token) = paused.as_ref() { registration.acknowledge(token).map_err(std::io::Error::other)?; }
                    // With no desktop, the host can exit once MCP has drained.
                    // With a desktop, the installer closes it after pause acknowledgement.
                    if app.get_webview_window("main").is_none() && (paused.is_some() || idle_since.elapsed()>Duration::from_secs(5)) {
                        app.exit(0); return Ok(());
                    }
                }
            }
        }
    }
}
