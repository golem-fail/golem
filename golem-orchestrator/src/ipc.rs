//! The orchestrator's socket protocol: one daemon per socket owns every
//! device lease, and each `golem` command is a client that submits its work
//! and waits for that work only. [`crate::daemon`] starts and stops the
//! daemon; this module is the server it runs and the client side of the
//! protocol.
//!
//! Protocol: JSON objects terminated by newline over a unix domain socket,
//! `~/.golem/golem.sock` unless `GOLEM_SOCKET` names another.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::suite::{SuiteConfig, SuiteRunner};

/// Build the orchestrator socket path under a supplied base directory,
/// creating the `.golem` directory if it does not yet exist.
///
/// Split out from [`socket_path`] so tests can inject a temp base instead
/// of touching the real `~/.golem`.
fn socket_path_in(base: &Path) -> PathBuf {
    let dir = base.join(".golem");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("golem.sock")
}

/// The orchestrator socket: `GOLEM_SOCKET`, else `~/.golem/golem.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GOLEM_SOCKET").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    socket_path_in(Path::new(&home))
}

/// Which golem a daemon or client is. A client and its daemon must match
/// exactly: the daemon installs the companion of its own version, and a
/// daemon left running by an earlier build of the same version would run
/// a client's flows with old code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub version: String,
    /// The executable's path, size and modification time, hashed: it
    /// changes with every build, even at an unchanged version.
    pub build: String,
    pub binary: String,
}

impl Identity {
    /// This process's identity.
    pub fn current() -> Self {
        let exe = std::env::current_exe().ok();
        let binary = exe
            .as_ref()
            .map_or_else(|| "unknown".to_string(), |p| p.display().to_string());
        let build = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            binary.hash(&mut h);
            if let Some(meta) = exe.as_ref().and_then(|p| std::fs::metadata(p).ok()) {
                meta.len().hash(&mut h);
                if let Ok(modified) = meta.modified() {
                    modified.hash(&mut h);
                }
            }
            format!("{:016x}", h.finish())
        };
        Identity {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build,
            binary,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "build": self.build,
            "binary": self.binary,
        })
    }

    fn from_json(v: &serde_json::Value) -> Self {
        let field = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        Identity {
            version: field("version"),
            build: field("build"),
            binary: field("binary"),
        }
    }
}

/// Whether version `a` is older than `b`, comparing dotted numeric parts.
pub(crate) fn version_older(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parts(a) < parts(b)
}

/// What a daemon answered to `hello`.
pub enum Hello {
    /// Same golem: the stream is ready for a `submit`.
    Ready(UnixStream),
    /// The daemon is finishing its work before it exits.
    Draining { daemon: Identity, runs: u64 },
    /// The daemon is an older version, or another build of this version.
    Stale {
        stream: UnixStream,
        daemon: Identity,
    },
    /// The daemon is a newer version than this client.
    Newer { daemon: Identity },
}

/// Why `hello` got no answer.
#[derive(Debug)]
pub enum HelloFailure {
    /// Nothing listens on the socket: no file, or a file nothing accepts on.
    Absent,
    /// Something accepted the connection but did not answer properly. It may
    /// be a daemon under load, so its socket is not stale.
    Unresponsive(anyhow::Error),
}

/// How long a daemon has to answer `hello` before it counts as
/// unresponsive.
pub const HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Introduce `me` to the daemon on `path`, which has `timeout` to answer.
pub async fn hello(
    path: &Path,
    me: &Identity,
    timeout: std::time::Duration,
) -> std::result::Result<Hello, HelloFailure> {
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(HelloFailure::Absent)
        }
        Err(e) => return Err(HelloFailure::Unresponsive(e.into())),
    };
    exchange_hello(stream, me, timeout)
        .await
        .map_err(HelloFailure::Unresponsive)
}

async fn exchange_hello(
    mut stream: UnixStream,
    me: &Identity,
    timeout: std::time::Duration,
) -> Result<Hello> {
    let mut msg = me.to_json();
    msg["type"] = serde_json::json!("hello");
    stream
        .write_all(format!("{msg}\n").as_bytes())
        .await
        .context("failed to send hello")?;
    let line = read_reply_line(&mut stream, timeout)
        .await
        .context("no answer to hello")?;
    let reply: serde_json::Value =
        serde_json::from_str(line.trim()).context("invalid hello reply")?;
    if reply["type"] == "error"
        && reply["message"]
            .as_str()
            .is_some_and(|m| m.contains("unknown message type"))
    {
        // A golem from before the handshake. It cannot drain, but it exits
        // once its own run is done; until then it holds devices.
        return Ok(Hello::Draining {
            daemon: Identity {
                version: "older".into(),
                build: String::new(),
                binary: "unknown".into(),
            },
            runs: 1,
        });
    }
    if reply["type"] != "hello" {
        anyhow::bail!("unexpected reply to hello: {line}");
    }
    let daemon = Identity::from_json(&reply);
    if reply["draining"].as_bool() == Some(true) {
        let runs = reply["runs"].as_u64().unwrap_or(0);
        return Ok(Hello::Draining { daemon, runs });
    }
    if daemon.version == me.version && daemon.build == me.build {
        return Ok(Hello::Ready(stream));
    }
    if version_older(&me.version, &daemon.version) {
        return Ok(Hello::Newer { daemon });
    }
    Ok(Hello::Stale { stream, daemon })
}

/// Ask the daemon on `stream` to finish its work and exit.
pub async fn drain(mut stream: UnixStream) -> Result<()> {
    stream
        .write_all(b"{\"type\":\"drain\"}\n")
        .await
        .context("failed to send drain")?;
    read_reply_line(&mut stream, HELLO_TIMEOUT)
        .await
        .context("no answer to drain")?;
    Ok(())
}

/// Read one newline-terminated line byte by byte, so nothing past it is
/// consumed from a stream the caller goes on using.
async fn read_reply_line(stream: &mut UnixStream, timeout: std::time::Duration) -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut line = Vec::new();
    let read = async {
        loop {
            let mut byte = [0u8; 1];
            if stream.read(&mut byte).await? == 0 {
                anyhow::bail!("connection closed");
            }
            if byte[0] == b'\n' {
                return Ok(());
            }
            line.push(byte[0]);
        }
    };
    tokio::time::timeout(timeout, read)
        .await
        .context("timed out")??;
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// Try to connect to the orchestrator listening on `path`.
///
/// Returns the connected stream if successful, or an error if no server
/// is running (socket doesn't exist or connection refused).
pub async fn try_connect(path: &Path) -> Result<UnixStream> {
    if !path.exists() {
        return Err(golem_events::coded(
            golem_events::FailureCode::HostOrchestratorIpc,
            anyhow::anyhow!("no socket at {}", path.display()),
        ));
    }

    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("failed to connect to {}", path.display()))?;

    // Verify the server is alive with a ping
    let mut stream = stream;
    let msg = serde_json::json!({"type": "ping"});
    stream
        .write_all(format!("{}\n", msg).as_bytes())
        .await
        .context("failed to send ping")?;

    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_line(&mut line),
    )
    .await
    .context("ping timeout")?
    .context("failed to read pong")?;

    if !line.contains("pong") {
        return Err(golem_events::coded(
            golem_events::FailureCode::HostOrchestratorIpc,
            anyhow::anyhow!("unexpected response to ping: {line}"),
        ));
    }

    // Reconnect since we consumed the stream in the ping check
    let stream = UnixStream::connect(path).await?;
    Ok(stream)
}

/// The orchestrator server.
///
/// Listens on a unix socket and handles client connections in a background
/// task. Every submit shares one ResourceManager AND one InstallCache, so
/// concurrent runs coordinate device allocation *and* skip install scripts
/// on devices where a previous submit already installed. Cache lifetime =
/// server lifetime.
pub struct OrchestratorServer {
    accept: tokio::task::JoinHandle<()>,
    path: PathBuf,
    /// False once [`stop_accepting`](Self::stop_accepting) has unlinked the
    /// socket: by then a new daemon may own a socket at the same path.
    owns_socket: std::sync::atomic::AtomicBool,
    shared: ServerShared,
    /// Count of connected clients.
    active_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// When the last client disconnected, or when the server started; `None`
    /// while a client is connected.
    idle_since: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
}

/// Counts a run while it lives.
pub(crate) struct RunGuard(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl RunGuard {
    fn new(counter: &std::sync::Arc<std::sync::atomic::AtomicU64>) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        RunGuard(counter.clone())
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// State every client handler shares.
#[derive(Clone)]
struct ServerShared {
    resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: golem_runner::installer::InstallCache,
    /// Set once any submit asks for `--keep-devices`: the daemon then leaves
    /// the devices it booted running when it exits.
    keep_devices: std::sync::Arc<std::sync::atomic::AtomicBool>,
    identity: std::sync::Arc<Identity>,
    /// Set by a newer client's `drain`: refuse new work, exit when the
    /// running work is done.
    draining: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Submits in progress.
    active_runs: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl OrchestratorServer {
    /// The resource manager every submit allocates devices from.
    pub fn resource_mgr(
        &self,
    ) -> &std::sync::Arc<golem_devices::resource_manager::ResourceManager> {
        &self.shared.resource_mgr
    }

    /// Whether any submit so far asked to keep the devices golem booted.
    pub fn keep_devices(&self) -> bool {
        self.shared
            .keep_devices
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Whether a newer client asked this daemon to drain and no run is
    /// left: time to exit.
    pub fn drained(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.shared.draining.load(Ordering::Acquire)
            && self.shared.active_runs.load(Ordering::Acquire) == 0
    }

    /// Hold a run open, as a submit in progress does. For tests.
    #[cfg(test)]
    pub(crate) fn begin_run(&self) -> RunGuard {
        RunGuard::new(&self.shared.active_runs)
    }

    /// How long no client has been connected; zero while one is.
    pub fn idle_for(&self) -> std::time::Duration {
        self.idle_since
            .lock()
            .ok()
            .and_then(|since| *since)
            .map_or(std::time::Duration::ZERO, |since| since.elapsed())
    }

    /// Stop accepting connections and unlink the socket, so a client that
    /// connects from now on finds no daemon. Clients already connected keep
    /// their handlers.
    pub fn stop_accepting(&self) {
        self.accept.abort();
        if self
            .owns_socket
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Wait until every connected client has disconnected.
    pub async fn wait_for_clients(&self) {
        use std::sync::atomic::Ordering;
        while self.active_clients.load(Ordering::Acquire) > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

impl Drop for OrchestratorServer {
    fn drop(&mut self) {
        self.stop_accepting();
    }
}

/// Start the orchestrator server on `path`.
///
/// Binds the unix socket and spawns a task that accepts connections and
/// handles their messages. Binding fails when a socket file already exists
/// at `path`: [`crate::daemon`] decides, under its lock, whether a leftover
/// socket is stale.
pub async fn start_server(path: &Path, identity: &Identity) -> Result<OrchestratorServer> {
    let listener = UnixListener::bind(path)
        .with_context(|| format!("failed to bind socket at {}", path.display()))?;

    let shared = ServerShared {
        resource_mgr: std::sync::Arc::new(golem_devices::resource_manager::ResourceManager::new(
            golem_devices::concurrency::ConcurrencyConfig::default(),
        )),
        install_cache: golem_runner::installer::InstallCache::new(),
        keep_devices: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        identity: std::sync::Arc::new(identity.clone()),
        draining: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        active_runs: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };
    let active_clients = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let idle_since = std::sync::Arc::new(std::sync::Mutex::new(Some(std::time::Instant::now())));

    let sh = shared.clone();
    let ac = active_clients.clone();
    let idle = idle_since.clone();
    let accept = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let sh = sh.clone();
                    let ac = ac.clone();
                    let idle = idle.clone();
                    ac.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    if let Ok(mut since) = idle.lock() {
                        *since = None;
                    }
                    tokio::spawn(async move {
                        handle_client(stream, &sh).await;
                        if ac.fetch_sub(1, std::sync::atomic::Ordering::AcqRel) == 1 {
                            if let Ok(mut since) = idle.lock() {
                                *since = Some(std::time::Instant::now());
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("  [orchestrator] accept error: {e}");
                    break;
                }
            }
        }
    });

    Ok(OrchestratorServer {
        accept,
        path: path.to_path_buf(),
        owns_socket: std::sync::atomic::AtomicBool::new(true),
        shared,
        active_clients,
        idle_since,
    })
}

/// Handle a single client connection.
async fn handle_client(stream: UnixStream, shared: &ServerShared) {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let writer = std::sync::Arc::new(tokio::sync::Mutex::new(writer));
    let mut line = String::new();
    // A client from before the handshake would submit relative paths and
    // no environment, which this daemon would resolve against its own.
    let mut greeted = false;

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // client disconnected
            Ok(_) => {
                let json: serde_json::Value = match serde_json::from_str(line.trim()) {
                    Ok(v) => v,
                    Err(e) => {
                        let resp = serde_json::json!({"type": "error", "message": format!("invalid JSON: {e}")});
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
                        continue;
                    }
                };

                match json["type"].as_str() {
                    Some("hello") => {
                        greeted = true;
                        let mut resp = shared.identity.to_json();
                        resp["type"] = serde_json::json!("hello");
                        resp["pid"] = serde_json::json!(std::process::id());
                        resp["draining"] = serde_json::json!(shared
                            .draining
                            .load(std::sync::atomic::Ordering::Acquire));
                        resp["runs"] = serde_json::json!(shared
                            .active_runs
                            .load(std::sync::atomic::Ordering::Acquire));
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                    }
                    Some("drain") => {
                        shared
                            .draining
                            .store(true, std::sync::atomic::Ordering::Release);
                        eprintln!("  [orchestrator] draining for a newer golem");
                        let mut w = writer.lock().await;
                        let _ = w.write_all(b"{\"type\":\"draining\"}\n").await;
                    }
                    Some("ping") => {
                        let mut w = writer.lock().await;
                        let _ = w.write_all(b"{\"type\":\"pong\"}\n").await;
                    }
                    Some("status") => {
                        let resp = serde_json::json!({
                            "type": "status",
                            "version": env!("CARGO_PKG_VERSION"),
                            "pid": std::process::id(),
                        });
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
                    }
                    Some("submit") if !greeted => {
                        let resp = serde_json::json!({
                            "type": "error",
                            "message": format!(
                                "this golem is older than the running daemon {} ({}); \
                                 upgrade it, or set GOLEM_SOCKET to use a separate daemon",
                                shared.identity.version, shared.identity.binary
                            ),
                        });
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                    }
                    Some("submit")
                        if shared.draining.load(std::sync::atomic::Ordering::Acquire) =>
                    {
                        let resp = serde_json::json!({
                            "type": "error",
                            "message": "the golem daemon is draining for a newer golem; run again",
                        });
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                    }
                    Some("submit") => {
                        let _run = RunGuard::new(&shared.active_runs);
                        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
                        let submit = handle_submit(&json, shared, &writer, cancel_rx);
                        tokio::pin!(submit);
                        // The client sends nothing while its suite runs, so
                        // end-of-stream means it is gone (Ctrl-C, killed):
                        // cancel the suite rather than run it for nobody.
                        let mut rest = String::new();
                        loop {
                            tokio::select! {
                                () = &mut submit => break,
                                read = reader.read_line(&mut rest) => match read {
                                    Ok(0) | Err(_) => {
                                        let _ = cancel_tx.send(true);
                                        submit.await;
                                        return;
                                    }
                                    Ok(_) => rest.clear(),
                                },
                            }
                        }
                    }
                    Some(other) => {
                        let resp = serde_json::json!({"type": "error", "message": format!("unknown message type: {other}")});
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
                    }
                    None => {
                        let resp =
                            serde_json::json!({"type": "error", "message": "missing 'type' field"});
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
                    }
                }
            }
            Err(e) => {
                // Client disconnects mid-read are normal (especially
                // in-process clients drop the socket as soon as
                // submit_and_wait returns). Stay quiet unless --debug.
                if golem_common::is_debug() {
                    eprintln!("  [orchestrator] read error: {e}");
                }
                break;
            }
        }
    }
}

/// The subset of `SuiteConfig` fields that are decoded purely from the
/// submit message's JSON `config` object. The remaining `SuiteConfig`
/// fields (project_apps, device_settings, project_record, stream_human)
/// come from non-JSON sources and are assembled at the call site.
struct SubmitConfigFields {
    platform_override: Option<golem_devices::Platform>,
    seed: Option<u64>,
    verbose: bool,
    debug: bool,
    no_perf: bool,
    no_clean: bool,
    no_teardown: bool,
    browser_headed: bool,
    keep_devices: bool,
    no_results: bool,
    start: Option<String>,
    output_dir: PathBuf,
    project_root: PathBuf,
    vars: Vec<(String, String)>,
    coverage_override: Option<golem_parser::CoverageStrategy>,
    a11y_override: Option<golem_parser::A11yLevel>,
    a11y_min_confidence_override: Option<f32>,
    rebuild: bool,
    no_build: bool,
    dev: bool,
    dev_port: u16,
    record: bool,
    no_record: bool,
    trace: bool,
    repeat: u32,
    max_concurrency: Option<usize>,
    max_device_wait: Option<std::time::Duration>,
    stub_fail_on_runs: Option<Vec<u32>>,
    profile: Option<String>,
}

/// Parse the submit message's `config` JSON object into the
/// JSON-derived `SuiteConfig` fields. Pure: no I/O except the
/// `project_root` default which reads `current_dir` when the field is
/// absent (mirroring the original inline logic exactly).
fn parse_submit_config(cfg: &serde_json::Value) -> SubmitConfigFields {
    let platform_override = cfg["platform"].as_str().and_then(|p| match p {
        "ios" => Some(golem_devices::Platform::Ios),
        "android" => Some(golem_devices::Platform::Android),
        _ => None,
    });
    let seed = cfg["seed"].as_u64();
    let verbose = cfg["verbose"].as_bool().unwrap_or(false);
    let debug = cfg["debug"].as_bool().unwrap_or(false);
    let no_perf = cfg["no_perf"].as_bool().unwrap_or(false);
    let no_clean = cfg["no_clean"].as_bool().unwrap_or(false);
    let no_teardown = cfg["no_teardown"].as_bool().unwrap_or(false);
    let browser_headed = cfg["browser_headed"].as_bool().unwrap_or(false);
    let keep_devices = cfg["keep_devices"].as_bool().unwrap_or(false);
    let no_results = cfg["no_results"].as_bool().unwrap_or(false);
    let start = cfg["start"].as_str().map(String::from);
    let output_dir: PathBuf = cfg["output_dir"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".golem/results"));
    let project_root: PathBuf = cfg["project_root"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let vars: Vec<(String, String)> = cfg["vars"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    let k = pair.first()?.as_str()?.to_string();
                    let v = pair.get(1)?.as_str()?.to_string();
                    Some((k, v))
                })
                .collect()
        })
        .unwrap_or_default();
    let coverage_override = cfg["coverage"].as_str().and_then(|c| match c {
        "one" => Some(golem_parser::CoverageStrategy::One),
        "min" => Some(golem_parser::CoverageStrategy::Min),
        "smart" => Some(golem_parser::CoverageStrategy::Smart),
        "full" => Some(golem_parser::CoverageStrategy::Full),
        _ => None,
    });
    let a11y_override = cfg["a11y"].as_str().and_then(|c| match c {
        "off" => Some(golem_parser::A11yLevel::Off),
        "critical" => Some(golem_parser::A11yLevel::Critical),
        "relaxed" => Some(golem_parser::A11yLevel::Relaxed),
        "strict" => Some(golem_parser::A11yLevel::Strict),
        _ => None,
    });
    let a11y_min_confidence_override = cfg["a11y_min_confidence"].as_f64().map(|v| v as f32);
    let rebuild = cfg["rebuild"].as_bool().unwrap_or(false);
    let no_build = cfg["no_build"].as_bool().unwrap_or(false);
    let dev = cfg["dev"].as_bool().unwrap_or(false);
    let dev_port = cfg["dev_port"].as_u64().unwrap_or(8081) as u16;
    let record = cfg["record"].as_bool().unwrap_or(false);
    let no_record = cfg["no_record"].as_bool().unwrap_or(false);
    let trace = cfg["trace"].as_bool().unwrap_or(false);
    let repeat = cfg["repeat"]
        .as_u64()
        .map(|n| n.clamp(1, 100) as u32)
        .unwrap_or(1);
    let max_device_wait = cfg["max_device_wait_ms"]
        .as_u64()
        .map(std::time::Duration::from_millis);
    // Stub mode: an array (possibly empty) activates stub mode; absent
    // (null / missing) means real devices. Values are 1-based run indices.
    let stub_fail_on_runs = cfg["stub_fail_on_runs"].as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_u64().map(|n| n as u32))
            .collect()
    });
    let max_concurrency = cfg["max_concurrency"].as_u64().map(|n| n as usize);
    let profile = cfg["profile"].as_str().map(str::to_string);

    SubmitConfigFields {
        platform_override,
        seed,
        verbose,
        debug,
        no_perf,
        no_clean,
        no_teardown,
        browser_headed,
        keep_devices,
        no_results,
        start,
        output_dir,
        project_root,
        vars,
        coverage_override,
        a11y_override,
        a11y_min_confidence_override,
        rebuild,
        no_build,
        dev,
        dev_port,
        record,
        no_record,
        trace,
        repeat,
        max_concurrency,
        max_device_wait,
        stub_fail_on_runs,
        profile,
    }
}

/// The client's environment and working directory from a submit's
/// `config`: `client_env` as `[[key, value], …]` and `client_cwd`. `None`
/// when the client sent no environment.
fn parse_child_env(cfg: &serde_json::Value) -> Option<golem_common::command::ChildEnv> {
    let vars = cfg["client_env"]
        .as_array()?
        .iter()
        .filter_map(|pair| {
            let pair = pair.as_array()?;
            Some((
                pair.first()?.as_str()?.to_string(),
                pair.get(1)?.as_str()?.to_string(),
            ))
        })
        .collect();
    Some(golem_common::command::ChildEnv {
        cwd: cfg["client_cwd"].as_str().map(PathBuf::from),
        vars,
    })
}

/// Handle a "submit" message: run the suite and stream events to the client.
async fn handle_submit(
    json: &serde_json::Value,
    shared: &ServerShared,
    writer: &std::sync::Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    let paths: Vec<PathBuf> = json["flow_paths"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default();

    if paths.is_empty() {
        let resp = serde_json::json!({"type": "error", "message": "no flow_paths provided"});
        let mut w = writer.lock().await;
        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
        return;
    }

    let cfg = &json["config"];
    let SubmitConfigFields {
        platform_override,
        seed,
        verbose,
        debug,
        no_perf,
        no_clean,
        no_teardown,
        browser_headed,
        keep_devices,
        no_results,
        start,
        output_dir,
        project_root,
        vars,
        coverage_override,
        a11y_override,
        a11y_min_confidence_override,
        rebuild,
        no_build,
        dev,
        dev_port,
        record,
        no_record,
        trace,
        repeat,
        max_concurrency,
        max_device_wait,
        stub_fail_on_runs,
        profile,
    } = parse_submit_config(cfg);
    if keep_devices {
        shared
            .keep_devices
            .store(true, std::sync::atomic::Ordering::Release);
    }

    // Re-read the project's golem.toml from the client's project_root so
    // apps pick up bundle IDs, install scripts, and device defaults the
    // CLI saw locally. `ProjectAppConfig` isn't `Serialize`, so
    // round-tripping through the wire isn't practical — this is cheaper
    // anyway (one TOML parse per submit).
    let (project_config, _) = match crate::project::ProjectConfig::load_from(&project_root) {
        Ok(pc) => pc,
        Err(e) => {
            let resp = serde_json::json!({
                "type": "error",
                "message": format!("failed to load golem.toml under {}: {e}", project_root.display()),
            });
            let mut w = writer.lock().await;
            let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
            return;
        }
    };

    // Create an event channel for streaming to the client.
    let (fwd_tx, fwd_rx) = golem_events::channel::event_channel();

    // Spawn a task that serializes events and writes them to the socket.
    let event_writer = writer.clone();
    let mut event_rx = fwd_rx.subscribe();
    drop(fwd_rx); // don't need the subscription factory after this
    let stream_handle = tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            let wire: golem_events::WireEvent = (&event).into();
            if let Ok(json_str) = serde_json::to_string(&wire) {
                let line = format!("{{\"type\":\"event\",\"event\":{json_str}}}\n");
                let mut w = event_writer.lock().await;
                if w.write_all(line.as_bytes()).await.is_err() {
                    break; // client disconnected
                }
            }
        }
    });

    let config = SuiteConfig {
        platform: platform_override,
        seed,
        verbose,
        debug,
        no_perf,
        no_clean,
        no_teardown,
        browser_headed,
        keep_devices,
        no_results,
        start,
        vars,
        output_dir,
        project_root,
        project_apps: project_config.apps,
        coverage_override,
        a11y_override,
        a11y_min_confidence_override,
        rebuild,
        no_build,
        dev,
        dev_port,
        device_settings: project_config.device_settings,
        record,
        no_record,
        project_record: project_config.options.record,
        trace,
        repeat,
        max_concurrency,
        max_device_wait,
        stub_fail_on_runs,
        profile,
        // Server doesn't do its own human streaming — client handles output.
        stream_human: false,
        child_env: parse_child_env(cfg).map(std::sync::Arc::new),
    };

    let mut runner = SuiteRunner::with_resource_manager(
        config,
        shared.resource_mgr.clone(),
        shared.install_cache.clone(),
    );
    runner.event_forwarder = Some(fwd_tx);
    runner.cancel = Some(cancel);

    // `no_results` is already in scope (consumed by SuiteConfig
    // above). Re-read from cfg avoids ordering coupling with the
    // SuiteConfig construction site.
    let no_results_for_write = cfg["no_results"].as_bool().unwrap_or(false);
    let include_junit = cfg["include_junit"].as_bool().unwrap_or(false);

    let result = runner.run_suite(&paths).await;
    // Drop the runner (and its forwarder sender) to close the event stream.
    drop(runner);
    let _ = stream_handle.await;

    // Server-side result-file writing. The daemon owns the FS (it
    // knows the client's output_dir and runs alongside the device
    // pool). Files written here include results.json / results.toon
    // / optionally results.xml, plus everything per-flow already
    // written under run_*/. Mirrors `main.rs`'s server-mode write
    // so daemon + standalone parity is preserved.
    let server_output_dir: PathBuf = cfg["output_dir"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".golem/results"));
    let resp = match result {
        Ok(report) => {
            if !no_results_for_write {
                if let Err(e) = golem_report::output::write_results_to_dir(
                    &report,
                    &server_output_dir,
                    include_junit,
                ) {
                    eprintln!("  [orchestrator] result-file write failed: {e:#}");
                }
            }
            serde_json::json!({
                "type": "done",
                "report": {
                    "total_duration_ms": report.total_duration_ms,
                    "flows": report.flows.iter().map(|f| {
                        serde_json::json!({
                            "flow_name": f.flow_name,
                            "success": f.success,
                            "warnings": f.warnings,
                            "duration_ms": f.duration_ms,
                            "device_name": f.device_name,
                            "seed": f.seed,
                        })
                    }).collect::<Vec<_>>(),
                    "output_dir": server_output_dir.display().to_string(),
                    "include_junit": include_junit,
                },
                "queue_wait": queue_wait_json(&golem_common::host_queue::queue_wait_stats()),
            })
        }
        Err(e) => {
            serde_json::json!({"type": "error", "message": format!("suite failed: {e:#}")})
        }
    };
    let mut w = writer.lock().await;
    let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
}

/// Submit work to a running orchestrator and wait for results.
///
/// Sends the flow paths and config, then reads a stream of events
/// followed by a final "done" message. Events are fed to a local
/// human renderer so the client controls its own output format.
/// Tuple-shaped return so callers can both inspect the materialised
/// suite report (for stdout-format rendering, flake tally, etc.) and
/// branch on overall pass/fail.
pub struct SubmitOutcome {
    pub report: golem_report::SuiteReport,
    pub all_passed: bool,
    /// The daemon's host-queue congestion over the run.
    pub queue_wait: golem_common::host_queue::QueueWaitStats,
}

/// The host-queue stats for the `done` message. They are process-global,
/// so they live in the daemon, not in the client that renders them.
fn queue_wait_json(stats: &golem_common::host_queue::QueueWaitStats) -> serde_json::Value {
    serde_json::json!({
        "total_us": stats.total.as_micros() as u64,
        "per_class": stats.per_class.iter().map(|c| serde_json::json!({
            "class": c.class.label(),
            "waited_us": c.waited.as_micros() as u64,
            "count": c.count,
        })).collect::<Vec<_>>(),
    })
}

/// The inverse of [`queue_wait_json`]. Classes this build does not know are
/// dropped from the breakdown but still count toward the total.
fn queue_wait_from_json(v: &serde_json::Value) -> golem_common::host_queue::QueueWaitStats {
    use golem_common::host_queue::{ClassWait, OpClass, QueueWaitStats};
    let per_class = v["per_class"]
        .as_array()
        .map(|classes| {
            classes
                .iter()
                .filter_map(|c| {
                    let label = c["class"].as_str()?;
                    let class = OpClass::ALL.iter().copied().find(|k| k.label() == label)?;
                    Some(ClassWait {
                        class,
                        waited: std::time::Duration::from_micros(c["waited_us"].as_u64()?),
                        count: c["count"].as_u64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    QueueWaitStats {
        per_class,
        total: std::time::Duration::from_micros(v["total_us"].as_u64().unwrap_or(0)),
    }
}

pub async fn submit_and_wait(
    mut stream: UnixStream,
    flow_paths: &[PathBuf],
    config: &serde_json::Value,
    verbose: bool,
    debug: bool,
    stream_human: bool,
) -> Result<SubmitOutcome> {
    let repeat = config["repeat"].as_u64().unwrap_or(1).max(1);
    let repeat_suffix = if repeat > 1 {
        format!(", {repeat} times")
    } else {
        String::new()
    };
    eprintln!(
        "  [orchestrator] client — submitting {} flow(s){repeat_suffix}",
        flow_paths.len()
    );

    // Send submit message
    let paths: Vec<String> = flow_paths.iter().map(|p| p.display().to_string()).collect();
    let msg = serde_json::json!({
        "type": "submit",
        "flow_paths": paths,
        "config": config,
    });
    stream
        .write_all(format!("{}\n", msg).as_bytes())
        .await
        .context("failed to send submit message")?;

    // Create local event channel for rendering.
    let (local_tx, local_rx) = golem_events::channel::event_channel();

    // Spawn local human renderer only when the user wants human
    // output. With `--output toon` (etc.) we skip the stream so
    // stderr stays quiet and the chosen non-human format lands on
    // stdout cleanly.
    let human_handle = if stream_human {
        let human_rx = local_rx.subscribe();
        Some(tokio::spawn(async move {
            golem_report::stream::stream_human(human_rx, verbose, true, debug).await;
        }))
    } else {
        None
    };

    // Spawn local accumulator.
    let accumulator = std::sync::Arc::new(tokio::sync::Mutex::new(
        golem_report::accumulator::ReportAccumulator::new(),
    ));
    let acc_clone = accumulator.clone();
    let acc_rx = local_rx.subscribe();
    let acc_handle = tokio::spawn(async move {
        golem_report::accumulator::accumulate_events(acc_rx, &acc_clone).await;
    });
    drop(local_rx);

    // The client sends absolute paths, because the daemon does not run in
    // this directory; paths under it are shown relative again, as the user
    // gave them.
    let cwd_prefix = std::env::current_dir()
        .ok()
        .map(|cwd| format!("{}/", cwd.display()));

    // Read streamed events and final result.
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    let mut all_passed = true;
    let queue_wait;

    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .await
            .context("lost connection to orchestrator")?;

        if line.is_empty() {
            return Err(golem_events::coded(
                golem_events::FailureCode::HostOrchestratorIpc,
                anyhow::anyhow!("orchestrator disconnected unexpectedly"),
            ));
        }

        let mut response: serde_json::Value =
            serde_json::from_str(line.trim()).context("invalid JSON from orchestrator")?;
        if let Some(prefix) = &cwd_prefix {
            relativize(&mut response, prefix);
        }

        match response["type"].as_str() {
            Some("event") => {
                // Deserialize and re-emit locally.
                if let Ok(wire) =
                    serde_json::from_value::<golem_events::WireEvent>(response["event"].clone())
                {
                    let event = wire.into_event();
                    local_tx.emit(event.device_id, event.kind);
                }
            }
            Some("done") => {
                queue_wait = queue_wait_from_json(&response["queue_wait"]);
                // Final result — check pass/fail.
                if let Some(flows) = response["report"]["flows"].as_array() {
                    for flow in flows {
                        if flow["success"].as_bool() != Some(true) {
                            all_passed = false;
                        }
                    }
                }
                // Mirror server-mode's `Results: ...` line so clients
                // running against a daemon get the same UX.
                let report = &response["report"];
                let server_output_dir = report["output_dir"].as_str().unwrap_or("");
                if !server_output_dir.is_empty() {
                    let include_junit = report["include_junit"].as_bool().unwrap_or(false);
                    let formats = if include_junit {
                        "json, toon, xml"
                    } else {
                        "json, toon"
                    };
                    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
                    if use_color {
                        let abs = std::fs::canonicalize(server_output_dir)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| server_output_dir.to_string());
                        let uri = file_uri_str(&abs);
                        eprintln!(
                            "             \x1b[2mResults: \x1b]8;;{uri}\x1b\\{server_output_dir}/\x1b]8;;\x1b\\  ({formats})\x1b[0m"
                        );
                    } else {
                        eprintln!("             Results: {server_output_dir}/  ({formats})");
                    }
                }
                break;
            }
            Some("error") => {
                let msg = response["message"].as_str().unwrap_or("unknown error");
                return Err(golem_events::coded(
                    golem_events::FailureCode::HostOrchestratorIpc,
                    anyhow::anyhow!("Orchestrator error: {msg}"),
                ));
            }
            _ => {
                // Ignore unknown message types for forward compatibility.
            }
        }
    }

    // Close event channel and wait for renderers.
    drop(local_tx);
    if let Some(h) = human_handle {
        let _ = h.await;
    }
    let _ = acc_handle.await;

    // Now safe to consume the accumulator: both readers above have
    // exited (broadcast channel closed when `local_tx` dropped). The
    // outer caller uses this report for stdout-format rendering and
    // exit-code logic — server-side file writes already happened
    // before the daemon emitted `done`.
    let acc = std::sync::Arc::try_unwrap(accumulator)
        .map_err(|_| anyhow::anyhow!("accumulator still has live refs"))?
        .into_inner();
    let report = acc.into_suite_report();
    Ok(SubmitOutcome {
        report,
        all_passed,
        queue_wait,
    })
}

/// Strip `prefix` from every string in `value` that starts with it.
fn relativize(value: &mut serde_json::Value, prefix: &str) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(rest) = s.strip_prefix(prefix) {
                *s = rest.to_string();
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| relativize(v, prefix)),
        serde_json::Value::Object(map) => map.values_mut().for_each(|v| relativize(v, prefix)),
        _ => {}
    }
}

/// Build a `file://` URI from a string path with percent-encoding so
/// spaces and non-ASCII characters don't break OSC 8 hyperlinks.
/// Mirror of `main.rs::file_uri` for a string input — kept duplicate
/// rather than re-extracting because both crates avoid taking on a
/// utility module just for this two-callsite helper.
fn file_uri_str(path: &str) -> String {
    let mut out = String::from("file://");
    for &c in path.as_bytes() {
        let unreserved = c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/');
        if unreserved {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. A plain ASCII alphanumeric path keeps every byte and gains only the scheme.
    #[test]
    fn file_uri_str_plain_ascii_is_unchanged_after_scheme() {
        let uri = file_uri_str("/Users/dev/results");
        assert_eq!(
            uri, "file:///Users/dev/results",
            "plain ASCII path SHALL be appended verbatim after file://"
        );
    }

    // 2. The unreserved set (- . _ ~ /) SHALL pass through without percent-encoding.
    #[test]
    fn file_uri_str_unreserved_chars_pass_through() {
        let uri = file_uri_str("/a-b/c.d/e_f/g~h/");
        assert_eq!(
            uri, "file:///a-b/c.d/e_f/g~h/",
            "unreserved chars -._~/ SHALL not be percent-encoded"
        );
    }

    // 3. A space is reserved and SHALL be percent-encoded as %20.
    #[test]
    fn file_uri_str_space_is_percent_encoded() {
        let uri = file_uri_str("/a b");
        assert_eq!(
            uri, "file:///a%20b",
            "a space SHALL be encoded as %20 so OSC 8 links don't break"
        );
    }

    // 4. Reserved ASCII punctuation (e.g. % itself) SHALL be percent-encoded uppercase hex.
    #[test]
    fn file_uri_str_reserved_punctuation_is_uppercase_hex() {
        let uri = file_uri_str("a%b:c");
        assert_eq!(
            uri, "file://a%25b%3Ac",
            "reserved punctuation SHALL be encoded as uppercase two-digit hex"
        );
    }

    // 5. Multibyte UTF-8 (non-ASCII) SHALL be encoded per-byte, not per-char.
    #[test]
    fn file_uri_str_non_ascii_is_encoded_per_byte() {
        let uri = file_uri_str("/r\u{00e9}sum\u{00e9}.txt");
        assert_eq!(
            uri, "file:///r%C3%A9sum%C3%A9.txt",
            "each UTF-8 byte of a non-ASCII char SHALL be percent-encoded"
        );
    }

    // 6. An empty path yields just the scheme prefix.
    #[test]
    fn file_uri_str_empty_path_is_scheme_only() {
        let uri = file_uri_str("");
        assert_eq!(uri, "file://", "empty input SHALL produce just the scheme");
    }

    // 7. An empty config object SHALL produce all defaults (bools false,
    //    repeat 1, default output dir, current dir as project root, no overrides).
    #[test]
    fn parse_submit_config_empty_object_uses_defaults() {
        let cfg = serde_json::json!({});
        let f = parse_submit_config(&cfg);
        assert!(
            f.platform_override.is_none(),
            "absent platform SHALL be None"
        );
        assert!(f.seed.is_none(), "absent seed SHALL be None");
        assert!(
            !f.verbose && !f.debug && !f.no_perf,
            "absent bools SHALL default false"
        );
        assert!(
            !f.no_clean && !f.no_teardown && !f.keep_devices && !f.no_results,
            "absent bools SHALL default false"
        );
        assert!(f.start.is_none(), "absent start SHALL be None");
        assert_eq!(
            f.output_dir,
            PathBuf::from(".golem/results"),
            "absent output_dir SHALL default to .golem/results"
        );
        assert!(f.vars.is_empty(), "absent vars SHALL be empty");
        assert!(
            f.coverage_override.is_none(),
            "absent coverage SHALL be None"
        );
        assert!(
            f.a11y_min_confidence_override.is_none(),
            "absent a11y_min_confidence SHALL be None"
        );
        assert!(!f.rebuild && !f.no_build && !f.record && !f.no_record && !f.trace);
        assert!(!f.dev, "absent dev SHALL default false");
        assert_eq!(
            f.dev_port, 8081,
            "absent dev_port SHALL default to Metro's 8081"
        );
        assert_eq!(f.repeat, 1, "absent repeat SHALL default to 1");
        assert!(
            f.max_device_wait.is_none(),
            "absent max_device_wait SHALL be None"
        );
    }

    // A `--dev` run crosses the daemon socket as config JSON. The flag is
    // useless to the server unless both halves agree on the key names, and a
    // field added only to the client silently vanishes here.
    #[test]
    fn parse_submit_config_carries_dev_mode() {
        let f = parse_submit_config(&serde_json::json!({"dev": true, "dev_port": 19000}));
        assert!(f.dev, "dev SHALL survive the wire");
        assert_eq!(f.dev_port, 19000, "dev_port SHALL survive the wire");
    }

    // 8. Platform strings map to the matching enum; unknown strings map to None.
    #[test]
    fn parse_submit_config_platform_enum_mapping() {
        let ios = parse_submit_config(&serde_json::json!({"platform": "ios"}));
        assert_eq!(
            ios.platform_override,
            Some(golem_devices::Platform::Ios),
            "\"ios\" SHALL map to Platform::Ios"
        );
        let android = parse_submit_config(&serde_json::json!({"platform": "android"}));
        assert_eq!(
            android.platform_override,
            Some(golem_devices::Platform::Android),
            "\"android\" SHALL map to Platform::Android"
        );
        let bogus = parse_submit_config(&serde_json::json!({"platform": "web"}));
        assert!(
            bogus.platform_override.is_none(),
            "an unknown platform string SHALL map to None"
        );
    }

    // 9. Coverage strings map to each strategy; unknown strings map to None.
    #[test]
    fn parse_submit_config_coverage_enum_mapping() {
        use golem_parser::CoverageStrategy;
        let cases = [
            ("one", CoverageStrategy::One),
            ("min", CoverageStrategy::Min),
            ("smart", CoverageStrategy::Smart),
            ("full", CoverageStrategy::Full),
        ];
        for (s, expected) in cases {
            let f = parse_submit_config(&serde_json::json!({ "coverage": s }));
            assert_eq!(
                f.coverage_override,
                Some(expected),
                "coverage \"{s}\" SHALL map to its strategy"
            );
        }
        let bogus = parse_submit_config(&serde_json::json!({"coverage": "none"}));
        assert!(
            bogus.coverage_override.is_none(),
            "an unknown coverage string SHALL map to None"
        );
    }

    // 9b. a11y_min_confidence round-trips off the wire as an f32; absent → None.
    #[test]
    fn parse_submit_config_a11y_min_confidence() {
        let set = parse_submit_config(&serde_json::json!({"a11y_min_confidence": 0.7}));
        assert_eq!(
            set.a11y_min_confidence_override,
            Some(0.7_f32),
            "a11y_min_confidence SHALL round-trip as an f32"
        );
        let absent = parse_submit_config(&serde_json::json!({}));
        assert!(
            absent.a11y_min_confidence_override.is_none(),
            "absent a11y_min_confidence SHALL be None"
        );
    }

    // 10. repeat is clamped into [1, 100]: 0 -> 1, in-range passes through, >100 -> 100.
    #[test]
    fn parse_submit_config_repeat_is_clamped() {
        let zero = parse_submit_config(&serde_json::json!({"repeat": 0}));
        assert_eq!(zero.repeat, 1, "repeat 0 SHALL clamp up to 1");
        let mid = parse_submit_config(&serde_json::json!({"repeat": 42}));
        assert_eq!(mid.repeat, 42, "an in-range repeat SHALL pass through");
        let over = parse_submit_config(&serde_json::json!({"repeat": 9999}));
        assert_eq!(over.repeat, 100, "repeat above 100 SHALL clamp down to 100");
    }

    // 11. max_device_wait_ms becomes a Duration of that many milliseconds.
    #[test]
    fn parse_submit_config_max_device_wait_is_millis() {
        let f = parse_submit_config(&serde_json::json!({"max_device_wait_ms": 2500}));
        assert_eq!(
            f.max_device_wait,
            Some(std::time::Duration::from_millis(2500)),
            "max_device_wait_ms SHALL be read as milliseconds"
        );
    }

    // 12. vars decode only well-formed [k, v] string pairs; malformed entries are dropped.
    #[test]
    fn parse_submit_config_vars_keep_only_string_pairs() {
        let cfg = serde_json::json!({
            "vars": [
                ["KEY", "VALUE"],
                ["ONLY_KEY"],
                [1, 2],
                "not-an-array",
                ["K2", "V2"]
            ]
        });
        let f = parse_submit_config(&cfg);
        assert_eq!(
            f.vars,
            vec![
                ("KEY".to_string(), "VALUE".to_string()),
                ("K2".to_string(), "V2".to_string())
            ],
            "only well-formed [string, string] pairs SHALL be kept"
        );
    }

    // 13. output_dir and project_root honour explicit string values from the config.
    #[test]
    fn parse_submit_config_paths_honour_explicit_values() {
        let cfg = serde_json::json!({
            "output_dir": "/tmp/out",
            "project_root": "/tmp/proj"
        });
        let f = parse_submit_config(&cfg);
        assert_eq!(
            f.output_dir,
            PathBuf::from("/tmp/out"),
            "explicit output_dir SHALL be used verbatim"
        );
        assert_eq!(
            f.project_root,
            PathBuf::from("/tmp/proj"),
            "explicit project_root SHALL be used verbatim"
        );
    }

    #[tokio::test]
    async fn a_submit_without_hello_is_refused() {
        let dir = tempfile::Builder::new()
            .prefix("gis")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let socket = dir.path().join("d.sock");
        let me = Identity {
            version: "1.2.3".into(),
            build: "b".into(),
            binary: "/opt/golem".into(),
        };
        let _server = start_server(&socket, &me).await.expect("server");
        let mut stream = UnixStream::connect(&socket).await.expect("connect");
        stream
            .write_all(b"{\"type\":\"submit\",\"flow_paths\":[\"x\"],\"config\":{}}\n")
            .await
            .expect("write");
        let reply = read_reply_line(&mut stream, HELLO_TIMEOUT)
            .await
            .expect("reply");
        assert!(
            reply.contains("older than the running daemon 1.2.3 (/opt/golem)"),
            "{reply}"
        );
        assert!(reply.contains("GOLEM_SOCKET"), "{reply}");
    }

    #[test]
    fn relativize_strips_the_prefix_from_nested_strings_only() {
        let mut v = serde_json::json!({
            "path": "/proj/.golem/results/a.png",
            "list": ["/proj/x", "/other/y", "/projx/z"],
            "n": 3,
            "msg": "wrote /proj/a",
        });
        relativize(&mut v, "/proj/");
        assert_eq!(
            v,
            serde_json::json!({
                "path": ".golem/results/a.png",
                "list": ["x", "/other/y", "/projx/z"],
                "n": 3,
                "msg": "wrote /proj/a",
            })
        );
    }

    #[test]
    fn queue_wait_stats_round_trip_the_wire() {
        use golem_common::host_queue::{ClassWait, OpClass, QueueWaitStats};
        let stats = QueueWaitStats {
            per_class: vec![ClassWait {
                class: OpClass::Install,
                waited: std::time::Duration::from_millis(1500),
                count: 2,
            }],
            total: std::time::Duration::from_millis(1500),
        };
        let back = queue_wait_from_json(&queue_wait_json(&stats));
        assert_eq!(back.total, stats.total);
        assert_eq!(back.per_class.len(), 1);
        assert_eq!(back.per_class[0].class, OpClass::Install);
        assert_eq!(back.per_class[0].count, 2);
        assert!(queue_wait_from_json(&serde_json::Value::Null).is_zero());
    }

    // 15. socket_path_in builds `<base>/.golem/golem.sock` under the
    //     supplied base and materializes the `.golem` directory.
    #[test]
    fn socket_path_in_builds_path_under_supplied_base() {
        let base = std::env::temp_dir().join(format!("golem-sockpath-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let sock = socket_path_in(&base);

        assert_eq!(
            sock,
            base.join(".golem").join("golem.sock"),
            "socket_path_in SHALL build <base>/.golem/golem.sock"
        );
        assert!(
            base.join(".golem").is_dir(),
            "socket_path_in SHALL create the .golem directory under the base"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
