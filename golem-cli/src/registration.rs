//! Companion registration server.
//!
//! Golem starts a lightweight HTTP server that companions call at startup
//! to register themselves and receive their port allocation. This replaces
//! port scanning and guessing.
//!
//! Flow:
//! 1. Golem starts registration server on a port in range 8220-8240
//! 2. Companions call POST /register with their device info
//! 3. Golem responds with the port the companion should serve on
//! 4. For Android: golem sets up ADB forward before responding

use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener as AsyncTcpListener;

/// Port range for the registration server itself.
const REG_PORT_START: u16 = 8220;
const REG_PORT_END: u16 = 8240;

/// Port range for companion servers (allocated by golem).
const COMPANION_PORT_START: u16 = 8250;
const COMPANION_PORT_END: u16 = 8499;

/// A registered companion.
#[derive(Debug, Clone)]
pub struct RegisteredCompanion {
    pub platform: String,
    pub device_id: String,
    pub device_name: String,
    pub version: String,
    pub port: u16,
}

/// Shared state for the registration server.
#[derive(Clone)]
pub struct RegistrationState {
    inner: Arc<Mutex<RegistrationInner>>,
    /// Per-UDID launch lock. When N flows want the same simulator, the
    /// first to take this lock kicks off `ensure_companion_with_reg`;
    /// the rest wait, and once they take the lock they find the
    /// already-registered companion via `get()` and return immediately.
    /// Without this, all N flows xcodebuild-launch the harness in
    /// parallel — only one xcodebuild instance can actually run, and
    /// the others "succeed" at spawning but their xcodebuild process
    /// exits silently, so they never see a registration event.
    launch_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Per-UDID "companion port is ready" cell. The first flow to ask
    /// drives the in-session cache check + spawn pipeline to
    /// completion; the rest await the same cell and resolve
    /// immediately once it's populated. Without this, N parallel
    /// flows would race past the `reg_state.get()` early-return
    /// before any of them registered, then all fall through to the
    /// spawn path. The per-UDID launch_guard serialised them but the
    /// losers still couldn't make progress because their target port
    /// was the one the winner had already claimed.
    companion_cells: Arc<Mutex<HashMap<String, Arc<tokio::sync::OnceCell<u16>>>>>,
}

struct RegistrationInner {
    companions: HashMap<String, RegisteredCompanion>, // device_id → companion
    next_port: u16,
    /// Notification channel: sends device_id when a companion registers.
    notify_tx: tokio::sync::broadcast::Sender<String>,
}

impl RegistrationState {
    pub fn new() -> (Self, tokio::sync::broadcast::Receiver<String>) {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        let state = Self {
            inner: Arc::new(Mutex::new(RegistrationInner {
                companions: HashMap::new(),
                next_port: COMPANION_PORT_START,
                notify_tx: tx,
            })),
            launch_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            companion_cells: Arc::new(Mutex::new(HashMap::new())),
        };
        (state, rx)
    }

    /// Get-or-init the companion port for a UDID. The `init` future runs
    /// at most once per UDID across the suite — all concurrent callers
    /// share the same OnceCell and wake when it's populated. If `init`
    /// errors, the cell stays empty so a subsequent call can retry.
    pub async fn ensure_companion_port<F, Fut>(&self, udid: &str, init: F) -> Result<u16>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<u16>>,
    {
        let cell = {
            let mut map = self.companion_cells.lock().expect("lock poisoned");
            map.entry(udid.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
                .clone()
        };
        cell.get_or_try_init(init).await.map(|p| *p)
    }

    /// Clear the cached companion port for a UDID. Used when a previously
    /// healthy companion fails — the next caller drives a fresh init.
    pub fn invalidate_companion(&self, udid: &str) {
        let mut map = self.companion_cells.lock().expect("lock poisoned");
        map.remove(udid);
    }

    /// Acquire a per-UDID lock guarding the launch path. Caller must
    /// hold the returned guard for the duration of the
    /// `ensure_companion_with_reg` call (or any equivalent flow that
    /// triggers `xcodebuild test-without-building` for that sim).
    pub async fn launch_guard(&self, udid: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = {
            let mut map = self.launch_locks.lock().await;
            map.entry(udid.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        mutex.lock_owned().await
    }

    /// Allocate a free port for a companion. Skips ports that are already
    /// in use (e.g. a companion left running from a previous golem session).
    fn allocate_port(&self, device_id: &str, platform: &str, device_name: &str, version: &str) -> u16 {
        let mut inner = self.inner.lock().expect("lock poisoned");

        // Find next free port, skipping any that are already bound
        let mut port = inner.next_port;
        let start = port;
        loop {
            if TcpListener::bind(format!("127.0.0.1:{port}")).is_ok() {
                break; // Port is free
            }
            port += 1;
            if port > COMPANION_PORT_END {
                port = COMPANION_PORT_START;
            }
            if port == start {
                break; // Wrapped around, use it anyway
            }
        }
        inner.next_port = port + 1;
        if inner.next_port > COMPANION_PORT_END {
            inner.next_port = COMPANION_PORT_START;
        }

        inner.companions.insert(
            device_id.to_string(),
            RegisteredCompanion {
                platform: platform.to_string(),
                device_id: device_id.to_string(),
                device_name: device_name.to_string(),
                version: version.to_string(),
                port,
            },
        );

        let _ = inner.notify_tx.send(device_id.to_string());
        port
    }

    /// Get a registered companion by device_id.
    pub fn get(&self, device_id: &str) -> Option<RegisteredCompanion> {
        let inner = self.inner.lock().expect("lock poisoned");
        inner.companions.get(device_id).cloned()
    }

    /// Remove a stale registration. Called when a previously-registered
    /// companion fails its health check, so subsequent reg-state hits
    /// fall through to the spawn path instead of routing to a dead port.
    pub fn remove(&self, device_id: &str) {
        let mut inner = self.inner.lock().expect("lock poisoned");
        inner.companions.remove(device_id);
    }

    /// Get all registered companions.
    pub fn all(&self) -> Vec<RegisteredCompanion> {
        let inner = self.inner.lock().expect("lock poisoned");
        inner.companions.values().cloned().collect()
    }

    /// Subscribe to registration notifications.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        let inner = self.inner.lock().expect("lock poisoned");
        inner.notify_tx.subscribe()
    }
}

/// Start the registration server. Returns the port it's listening on.
pub async fn start_registration_server(state: RegistrationState) -> Result<u16> {
    // Find a free port in the registration range
    let (listener, port) = find_free_port_in_range(REG_PORT_START, REG_PORT_END)?;
    listener.set_nonblocking(true)?;
    let listener = AsyncTcpListener::from_std(listener)?;

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, state).await {
                    eprintln!("  [registration] error: {e}");
                }
            });
        }
    });

    Ok(port)
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    state: RegistrationState,
) -> Result<()> {
    let (reader, mut writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);

    // Read request line
    let mut request_line = String::new();
    buf_reader.read_line(&mut request_line).await?;

    // Read headers
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        buf_reader.read_line(&mut header).await?;
        if header.trim().is_empty() {
            break;
        }
        if header.to_lowercase().starts_with("content-length:") {
            content_length = header
                .split(':')
                .nth(1)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
        }
    }

    // Read body
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        tokio::io::AsyncReadExt::read_exact(&mut buf_reader, &mut body).await?;
    }

    // Route
    if request_line.starts_with("POST /register") {
        let body_str = String::from_utf8_lossy(&body);
        let req: serde_json::Value = serde_json::from_str(&body_str)
            .context("invalid JSON in /register body")?;

        let platform = req.get("platform").and_then(|v| v.as_str()).unwrap_or("");
        let device_id = req.get("device_id").and_then(|v| v.as_str()).unwrap_or("unknown");
        let device_name = req.get("device_name").and_then(|v| v.as_str()).unwrap_or("");
        let version = req.get("version").and_then(|v| v.as_str()).unwrap_or("");

        let port = state.allocate_port(device_id, platform, device_name, version);

        eprintln!("  [registration] {device_name} ({platform}) registered on port {port}");

        let response_body = serde_json::json!({ "port": port }).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        writer.write_all(response.as_bytes()).await?;
    } else {
        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        writer.write_all(response.as_bytes()).await?;
    }

    Ok(())
}

/// Find a free port in a range by trying to bind.
fn find_free_port_in_range(start: u16, end: u16) -> Result<(std::net::TcpListener, u16)> {
    for port in start..=end {
        if let Ok(listener) = TcpListener::bind(format!("127.0.0.1:{port}")) {
            return Ok((listener, port));
        }
    }
    Err(golem_events::coded(
        golem_events::FailureCode::HostPortsExhausted,
        anyhow::anyhow!("No free port in range {start}-{end}"),
    ))
}
