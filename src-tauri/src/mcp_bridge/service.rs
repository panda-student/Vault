//! One MCP core per Windows identity, executable path and canonical vault directory.
use super::{control, pipe};
use fs2::FileExt;
use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
pub fn run(
    executable: &Path,
    directory: &Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let base = control::directory(executable)?;
    let (name, state) = pipe::identity(&base, directory)?;
    fs::create_dir_all(&state)?;
    let lease = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state.join("service.lock"))?;
    if lease.try_lock_exclusive().is_err() {
        return Ok(());
    } // Another client already started the shared core.
    if control::active(&base).is_some() {
        return Ok(());
    }
    let registration = control::Registration::new(&base)?;
    let runtime = crate::core::UpdateRuntime::acquire(executable)?;
    let core = crate::core::open(directory, true)?;
    core.bind_update_runtime(runtime.clone())?;
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let mut listener = pipe::create(&name, true)?;
        let active = Arc::new(AtomicUsize::new(0));
        let mut idle_since = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_millis(75));
        loop {
            tokio::select! {
                connected = listener.connect() => {
                    connected?;
                    let next = pipe::create(&name, false)?;
                    let connection = std::mem::replace(&mut listener, next);
                    let core = core.clone();
                    let active = active.clone();
                    active.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async move {
                        struct ClientCount(Arc<AtomicUsize>);
                        impl Drop for ClientCount { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
                        let _count = ClientCount(active);
                        let (read, write) = tokio::io::split(connection);
                        let _ = crate::mcp::serve_connection(core, read, write).await;
                    });
                }
                _ = tick.tick() => {
                    if active.load(Ordering::SeqCst) > 0 { idle_since = Instant::now(); }
                    else if control::active(&base).is_some() || idle_since.elapsed() > Duration::from_secs(5) { break; }
                }
            }
        }
        Ok::<(), std::io::Error>(())
    })?;
    // All connection handlers are gone before acknowledging. No storage/core lease
    // remains while the updater replaces the image and possibly migrates data.
    drop(core);
    drop(runtime);
    if let Some(token) = control::active(&base) {
        registration.acknowledge(&token)?;
    }
    Ok(())
}
