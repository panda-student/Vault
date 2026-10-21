//! Stable stdio adapter. Credentials and business operations stay in the worker's
//! existing MCP adapter and Rust core. No write request is ever replayed.
pub mod control;
mod pipe;
pub mod service;
use fs2::FileExt;
use serde_json::{Value, json};
use std::{collections::HashMap, fs::OpenOptions, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader, WriteHalf},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    process::Command,
    sync::mpsc,
};
const MAX_FRAME: usize = 16 * 1024 * 1024;
enum Event {
    Client(Vec<u8>),
    Worker(u64, Vec<u8>),
    ClientEnd,
    WorkerEnd(u64),
}
async fn reader<R: AsyncRead + Unpin>(
    input: R,
    sender: mpsc::Sender<Event>,
    generation: Option<u64>,
) {
    use tokio::io::AsyncReadExt;
    let mut input = BufReader::new(input);
    loop {
        let mut line = Vec::new();
        let read = (&mut input)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut line)
            .await;
        if !matches!(read, Ok(n) if n > 0 && n <= MAX_FRAME) {
            break;
        }
        let event = match generation {
            Some(g) => Event::Worker(g, line),
            None => Event::Client(line),
        };
        if sender.send(event).await.is_err() {
            return;
        }
    }
    let _ = sender
        .send(match generation {
            Some(g) => Event::WorkerEnd(g),
            None => Event::ClientEnd,
        })
        .await;
}
struct Worker {
    input: Option<WriteHalf<NamedPipeClient>>,
    reader: tokio::task::JoinHandle<()>,
    bootstrap: Option<String>,
}
impl Worker {
    async fn start(
        executable: &Path,
        directory: &Path,
        generation: u64,
        sender: mpsc::Sender<Event>,
        initialize: Option<&Value>,
    ) -> std::io::Result<Self> {
        let base = control::directory(executable).map_err(std::io::Error::other)?;
        let (name, state) = pipe::identity(&base, directory)?;
        std::fs::create_dir_all(&state)?;
        let until = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut last_launch = tokio::time::Instant::now() - Duration::from_secs(2);
        let connection = loop {
            if control::active(&base).is_some() {
                return Err(std::io::Error::other("Update in progress"));
            }
            if let Ok(connection) = ClientOptions::new().open(&name) {
                pipe::verify_server(&connection)?;
                break connection;
            }
            // Serialize startup across all connection bridges. A live service lease
            // prevents duplicate cores even if multiple clients connect concurrently.
            let startup = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(state.join("startup.lock"))?;
            if last_launch.elapsed() > Duration::from_secs(1)
                && startup.try_lock_exclusive().is_ok()
            {
                let lease = OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(state.join("service.lock"))?;
                if lease.try_lock_exclusive().is_ok() {
                    let mut command = Command::new(executable);
                    command
                        .arg("--mcp-service")
                        .arg("--data-dir")
                        .arg(directory)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .creation_flags(0x08000000);
                    drop(lease);
                    command.spawn()?;
                    last_launch = tokio::time::Instant::now();
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            drop(startup);
            if tokio::time::Instant::now() >= until {
                return Err(std::io::Error::other("Shared core did not become ready"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let (output, input) = tokio::io::split(connection);
        let task = tokio::spawn(reader(output, sender, Some(generation)));
        let mut worker = Self {
            input: Some(input),
            reader: task,
            bootstrap: None,
        };
        if let Some(initialize) = initialize {
            let mut request = initialize.clone();
            let id = format!("vault-resume-{}", uuid::Uuid::new_v4());
            request["id"] = Value::String(id.clone());
            worker.bootstrap = Some(id);
            worker.send(&serde_json::to_vec(&request)?).await?;
        }
        Ok(worker)
    }
    async fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Worker paused"))?;
        input.write_all(bytes).await?;
        if !bytes.ends_with(b"\n") {
            input.write_all(b"\n").await?;
        }
        input.flush().await
    }
    async fn stop(mut self) {
        // Named pipes have no write-half EOF: release both halves, after draining.
        self.reader.abort();
        self.input.take();
        let _ = self.reader.await;
    }
}
fn paused(id: &Value, uncertain: bool) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":if uncertain {
        "MCP service stopped before its response. Outcome unknown; inspect current state before retrying."
    } else { "UPDATE_IN_PROGRESS: MCP service is temporarily paused; retry after the update. No operation was executed." }}})
}
async fn output(stdout: &mut tokio::io::Stdout, value: &Value) -> std::io::Result<()> {
    stdout.write_all(&serde_json::to_vec(value)?).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await
}
pub fn run(
    executable: &Path,
    directory: &Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(serve(executable, directory))
}
async fn serve(
    executable: &Path,
    directory: &Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let base = control::directory(executable)?;
    let registration = control::Registration::new(&base)?;
    let (sender, mut receiver) = mpsc::channel(8);
    tokio::spawn(reader(tokio::io::stdin(), sender.clone(), None));
    let mut stdout = tokio::io::stdout();
    let mut generation = 0;
    let mut worker = if control::active(&base).is_none() {
        generation = 1;
        Some(Worker::start(executable, directory, generation, sender.clone(), None).await?)
    } else {
        None
    };
    let mut initialize: Option<Value> = None;
    let mut initialized = false;
    let mut pending: HashMap<String, Value> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(75));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Some(token) = control::active(&base) {
                    if pending.is_empty() {
                        if let Some(old) = worker.take() { old.stop().await; }
                        registration.acknowledge(&token)?;
                    }
                } else if worker.is_none() {
                    generation += 1;
                    // The canonical launch path now points at the installed or rolled-back binary.
                    // A short launch failure leaves the client connection alive and retryable.
                    if let Ok(next) = Worker::start(executable, directory, generation, sender.clone(), initialize.as_ref()).await {
                        worker = Some(next);
                    } else { tokio::time::sleep(Duration::from_millis(200)).await; }
                }
            }
            event = receiver.recv() => match event {
                Some(Event::ClientEnd) | None => break,
                Some(Event::Client(bytes)) => {
                    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
                    let id = value.get("id");
                    let method = value.get("method").and_then(Value::as_str);
                    // During a pause, reject new requests explicitly. In particular, never
                    // buffer and retry mutating operations after a worker restart.
                    let unavailable = control::active(&base).is_some() || worker.as_ref().is_none_or(|w| w.bootstrap.is_some());
                    if unavailable {
                        if let Some(id) = id { output(&mut stdout, &paused(id, false)).await?; }
                        continue;
                    }
                    if method == Some("initialize") { initialize = Some(value.clone()); }
                    if method == Some("notifications/initialized") { initialized = true; }
                    if method.is_some() {
                        if let Some(id) = id { pending.insert(id.to_string(), id.clone()); }
                    }
                    if worker.as_mut().unwrap().send(&bytes).await.is_err() {
                        for id in pending.values() { output(&mut stdout, &paused(id, true)).await?; }
                        pending.clear();
                        if let Some(old) = worker.take() { old.stop().await; }
                    }
                }
                Some(Event::Worker(g, bytes)) if g == generation => {
                    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { continue };
                    let bootstrap = worker.as_ref().and_then(|w| w.bootstrap.as_deref());
                    if bootstrap.is_some() && value.get("id").and_then(Value::as_str) == bootstrap {
                        if value.get("error").is_some() {
                            if let Some(old) = worker.take() { old.stop().await; }
                        } else if let Some(w) = worker.as_mut() {
                            w.bootstrap = None;
                            if initialized { w.send(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}").await?; }
                        }
                        continue;
                    }
                    if let Some(id) = value.get("id") { pending.remove(&id.to_string()); }
                    // Only forward worker output: it already passed the core's SafeWriter.
                    stdout.write_all(&bytes).await?; stdout.flush().await?;
                }
                Some(Event::WorkerEnd(g)) if g == generation => {
                    for id in pending.values() { output(&mut stdout, &paused(id, true)).await?; }
                    pending.clear();
                    if let Some(old) = worker.take() { old.stop().await; }
                }
                _ => {}
            }
        }
    }
    if let Some(old) = worker {
        old.stop().await;
    }
    Ok(())
}
