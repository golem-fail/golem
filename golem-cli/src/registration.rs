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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // 1. A fresh state has no companions; new() hands back a live receiver
    //    that observes a subsequent registration notification.
    #[tokio::test]
    async fn new_state_is_empty_and_notifies_receiver() {
        let (state, mut rx) = RegistrationState::new();

        assert!(state.all().is_empty(), "fresh state SHALL hold no companions");
        assert!(
            state.get("anything").is_none(),
            "fresh state SHALL return None for any device_id"
        );

        let port = state.allocate_port("dev-1", "ios", "iPhone", "1.0.0");
        let notified = rx.recv().await.expect("receiver SHALL get a notification");

        assert_eq!(notified, "dev-1", "notification SHALL carry the device_id");
        assert!(
            (COMPANION_PORT_START..=COMPANION_PORT_END).contains(&port),
            "allocated port SHALL be within the companion range"
        );
    }

    // 2. allocate_port records the companion's fields verbatim and makes it
    //    retrievable by device_id via get().
    #[test]
    fn allocate_port_records_companion_fields() {
        let (state, _rx) = RegistrationState::new();

        let port = state.allocate_port("dev-2", "android", "Pixel", "2.3.4");
        let got = state.get("dev-2").expect("companion SHALL be retrievable");

        assert_eq!(got.platform, "android", "platform SHALL be stored");
        assert_eq!(got.device_id, "dev-2", "device_id SHALL be stored");
        assert_eq!(got.device_name, "Pixel", "device_name SHALL be stored");
        assert_eq!(got.version, "2.3.4", "version SHALL be stored");
        assert_eq!(got.port, port, "stored port SHALL match the returned port");
    }

    // 3. Two distinct registrations get distinct, monotonically advancing
    //    ports (next_port advances past each allocation).
    #[test]
    fn allocate_port_advances_for_distinct_devices() {
        let (state, _rx) = RegistrationState::new();

        let p1 = state.allocate_port("dev-a", "ios", "A", "1");
        let p2 = state.allocate_port("dev-b", "ios", "B", "1");

        assert_ne!(p1, p2, "distinct devices SHALL receive distinct ports");
        assert!(p2 > p1, "next allocation SHALL advance past the previous port");
    }

    // 4. Re-registering the same device_id overwrites the prior entry rather
    //    than creating a duplicate; all() reflects a single companion.
    #[test]
    fn reregister_same_device_overwrites_entry() {
        let (state, _rx) = RegistrationState::new();

        state.allocate_port("dev-3", "ios", "Old", "1.0");
        state.allocate_port("dev-3", "android", "New", "2.0");
        let got = state.get("dev-3").expect("companion SHALL exist after re-register");

        assert_eq!(state.all().len(), 1, "re-register SHALL NOT create a duplicate");
        assert_eq!(got.device_name, "New", "later registration SHALL overwrite the entry");
        assert_eq!(got.platform, "android", "later platform SHALL win");
    }

    // 5. remove() drops a registration so subsequent get() returns None and
    //    all() no longer lists it.
    #[test]
    fn remove_drops_registration() {
        let (state, _rx) = RegistrationState::new();
        state.allocate_port("dev-4", "ios", "X", "1");

        state.remove("dev-4");

        assert!(state.get("dev-4").is_none(), "removed device SHALL be gone from get()");
        assert!(state.all().is_empty(), "removed device SHALL be gone from all()");
    }

    // 6. remove() on an unknown device_id is a no-op and SHALL NOT disturb
    //    existing registrations.
    #[test]
    fn remove_unknown_device_is_noop() {
        let (state, _rx) = RegistrationState::new();
        state.allocate_port("keep", "ios", "X", "1");

        state.remove("never-registered");

        assert!(state.get("keep").is_some(), "unrelated companion SHALL survive");
        assert_eq!(state.all().len(), 1, "no-op remove SHALL NOT change the count");
    }

    // 7. all() returns every distinct registration (order-independent).
    #[test]
    fn all_returns_every_companion() {
        let (state, _rx) = RegistrationState::new();
        state.allocate_port("d1", "ios", "A", "1");
        state.allocate_port("d2", "android", "B", "1");
        state.allocate_port("d3", "ios", "C", "1");

        let mut ids: Vec<String> = state.all().into_iter().map(|c| c.device_id).collect();
        ids.sort();

        assert_eq!(ids, vec!["d1", "d2", "d3"], "all() SHALL list every device_id");
    }

    // 8. A receiver obtained via subscribe() (after construction) still sees
    //    registration notifications.
    #[tokio::test]
    async fn subscribe_receives_notifications() {
        let (state, _rx) = RegistrationState::new();
        let mut sub = state.subscribe();

        state.allocate_port("late-sub", "ios", "X", "1");
        let got = sub.recv().await.expect("subscriber SHALL receive a notification");

        assert_eq!(got, "late-sub", "subscriber SHALL receive the registering device_id");
    }

    // 9. ensure_companion_port runs init exactly once for a UDID and shares
    //    the result across repeated calls.
    #[tokio::test]
    async fn ensure_companion_port_runs_init_once() {
        let (state, _rx) = RegistrationState::new();
        let calls = AtomicUsize::new(0);

        let first = state
            .ensure_companion_port("udid-1", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(8300)
            })
            .await
            .expect("first init SHALL succeed");
        let second = state
            .ensure_companion_port("udid-1", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(9999)
            })
            .await
            .expect("second call SHALL resolve from the cell");

        assert_eq!(first, 8300, "first call SHALL return the init value");
        assert_eq!(second, 8300, "second call SHALL return the cached value");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "init SHALL run at most once per UDID");
    }

    // 10. Distinct UDIDs each drive their own init independently.
    #[tokio::test]
    async fn ensure_companion_port_is_per_udid() {
        let (state, _rx) = RegistrationState::new();

        let a = state
            .ensure_companion_port("udid-a", || async { Ok(8310) })
            .await
            .expect("udid-a init SHALL succeed");
        let b = state
            .ensure_companion_port("udid-b", || async { Ok(8320) })
            .await
            .expect("udid-b init SHALL succeed");

        assert_eq!(a, 8310, "udid-a SHALL get its own value");
        assert_eq!(b, 8320, "udid-b SHALL get its own value");
    }

    // 11. When init errors, the cell stays empty so a later call can retry
    //    and succeed.
    #[tokio::test]
    async fn ensure_companion_port_retries_after_error() {
        let (state, _rx) = RegistrationState::new();

        let err = state
            .ensure_companion_port("udid-r", || async {
                Err(anyhow::anyhow!("boom"))
            })
            .await;
        assert!(err.is_err(), "failing init SHALL surface the error");

        let ok = state
            .ensure_companion_port("udid-r", || async { Ok(8330) })
            .await
            .expect("retry after error SHALL succeed");
        assert_eq!(ok, 8330, "cell SHALL stay empty after an error so retry can init");
    }

    // 12. invalidate_companion clears a populated cell so the next call
    //    re-runs init with a fresh value.
    #[tokio::test]
    async fn invalidate_companion_forces_reinit() {
        let (state, _rx) = RegistrationState::new();
        let first = state
            .ensure_companion_port("udid-i", || async { Ok(8340) })
            .await
            .expect("first init SHALL succeed");

        state.invalidate_companion("udid-i");

        let second = state
            .ensure_companion_port("udid-i", || async { Ok(8341) })
            .await
            .expect("post-invalidate init SHALL run again");
        assert_eq!(first, 8340, "pre-invalidate value SHALL be the original");
        assert_eq!(second, 8341, "after invalidate the next init SHALL produce a fresh value");
    }

    // 13. invalidate_companion on an unknown UDID leaves an unrelated,
    //     already-populated cell intact (the next call resolves from cache
    //     without re-running init).
    #[tokio::test]
    async fn invalidate_unknown_udid_is_noop() {
        let (state, _rx) = RegistrationState::new();
        let calls = AtomicUsize::new(0);

        // 1. Populate a cell for an unrelated UDID.
        let first = state
            .ensure_companion_port("kept-udid", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(8350)
            })
            .await
            .expect("first init SHALL succeed");

        // 2. Invalidate a UDID that was never seen.
        state.invalidate_companion("never-seen");

        // 3. The unrelated cell SHALL survive: a follow-up resolves from
        //    cache and does NOT re-run init.
        let second = state
            .ensure_companion_port("kept-udid", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(9999)
            })
            .await
            .expect("follow-up SHALL resolve from the surviving cell");

        assert_eq!(first, 8350, "first init SHALL return its value");
        assert_eq!(
            second, 8350,
            "unknown-UDID invalidate SHALL leave the unrelated cached value untouched"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "unknown-UDID invalidate SHALL NOT clear an unrelated cell, so init runs only once"
        );
    }

    // 14. launch_guard returns the same underlying mutex for a UDID, so a
    //    second acquire blocks until the first guard is dropped.
    #[tokio::test]
    async fn launch_guard_serialises_same_udid() {
        let (state, _rx) = RegistrationState::new();
        let guard = state.launch_guard("sim-1").await;

        // While holding the guard, a second acquire SHALL NOT complete.
        let second = state.launch_guard("sim-1");
        tokio::pin!(second);
        let pending = tokio::time::timeout(std::time::Duration::from_millis(50), &mut second).await;
        assert!(pending.is_err(), "second acquire of same UDID SHALL block while guard is held");

        drop(guard);
        let _g2 = second.await;
    }

    // 15. launch_guard for distinct UDIDs grants both immediately (no
    //    cross-UDID contention).
    #[tokio::test]
    async fn launch_guard_distinct_udids_dont_contend() {
        let (state, _rx) = RegistrationState::new();
        let _g1 = state.launch_guard("sim-x").await;

        let g2 = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            state.launch_guard("sim-y"),
        )
        .await;

        assert!(g2.is_ok(), "distinct UDID SHALL acquire its guard without blocking");
    }

    // 16. find_free_port_in_range returns the first bindable port in range.
    #[test]
    fn find_free_port_returns_port_in_range() {
        let (listener, port) =
            find_free_port_in_range(REG_PORT_START, REG_PORT_END).expect("a free reg port SHALL exist");

        assert!(
            (REG_PORT_START..=REG_PORT_END).contains(&port),
            "returned port SHALL fall within the requested range"
        );
        assert_eq!(
            listener.local_addr().expect("listener SHALL have an address").port(),
            port,
            "returned listener SHALL be bound to the returned port"
        );
    }

    // 17. When the whole range is occupied, find_free_port_in_range fails
    //    with the HostPortsExhausted failure code.
    #[test]
    fn find_free_port_exhausted_yields_coded_error() {
        // Occupy a tiny range entirely by holding listeners on each port.
        let mut held = Vec::new();
        let mut lo = None;
        let mut hi = None;
        // Find two adjacent free ports we can fully occupy.
        for p in 8400u16..8499 {
            if let Ok(l) = TcpListener::bind(format!("127.0.0.1:{p}")) {
                if lo.is_none() {
                    lo = Some(p);
                    held.push(l);
                } else if held.len() == 1 && p == lo.expect("lo set") + 1 {
                    hi = Some(p);
                    held.push(l);
                    break;
                } else {
                    // Not adjacent; reset and try from here.
                    lo = Some(p);
                    hi = None;
                    held.clear();
                    held.push(l);
                }
            } else {
                lo = None;
                hi = None;
                held.clear();
            }
        }
        let (lo, hi) = (lo.expect("found lo"), hi.expect("found two adjacent free ports"));

        let err = match find_free_port_in_range(lo, hi) {
            Ok(_) => panic!("a fully-occupied range SHALL yield an error"),
            Err(e) => e,
        };
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::HostPortsExhausted),
            "exhausted range SHALL carry the HostPortsExhausted code"
        );
    }

    // 18. End-to-end: a real /register POST against a started server returns
    //    the allocated port and records the companion in shared state.
    #[tokio::test]
    async fn register_post_allocates_and_records() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (state, _rx) = RegistrationState::new();
        let reg_port = start_registration_server(state.clone())
            .await
            .expect("registration server SHALL start");

        let body = serde_json::json!({
            "platform": "ios",
            "device_id": "e2e-dev",
            "device_name": "SimPhone",
            "version": "9.9.9",
        })
        .to_string();
        let request = format!(
            "POST /register HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let mut conn = tokio::net::TcpStream::connect(("127.0.0.1", reg_port))
            .await
            .expect("client SHALL connect to reg server");
        conn.write_all(request.as_bytes()).await.expect("client SHALL send request");
        let mut response = String::new();
        conn.read_to_string(&mut response).await.expect("client SHALL read response");

        assert!(response.starts_with("HTTP/1.1 200 OK"), "register SHALL return 200");
        let got = state.get("e2e-dev").expect("companion SHALL be recorded in state");
        assert!(
            response.contains(&format!("\"port\":{}", got.port)),
            "response body SHALL echo the allocated port"
        );
        assert_eq!(got.device_name, "SimPhone", "recorded device_name SHALL match the request");
    }

    // 19. An unknown route returns 404 and records nothing.
    #[tokio::test]
    async fn unknown_route_returns_404() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (state, _rx) = RegistrationState::new();
        let reg_port = start_registration_server(state.clone())
            .await
            .expect("registration server SHALL start");

        let request = "GET /nope HTTP/1.1\r\n\r\n";
        let mut conn = tokio::net::TcpStream::connect(("127.0.0.1", reg_port))
            .await
            .expect("client SHALL connect");
        conn.write_all(request.as_bytes()).await.expect("client SHALL send request");
        let mut response = String::new();
        conn.read_to_string(&mut response).await.expect("client SHALL read response");

        assert!(response.starts_with("HTTP/1.1 404 Not Found"), "unknown route SHALL 404");
        assert!(state.all().is_empty(), "non-register request SHALL record nothing");
    }

    // 20. A /register POST with missing fields falls back to the defaults
    //    (device_id "unknown", empty platform/name/version).
    #[tokio::test]
    async fn register_post_applies_field_defaults() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (state, _rx) = RegistrationState::new();
        let reg_port = start_registration_server(state.clone())
            .await
            .expect("registration server SHALL start");

        let body = "{}".to_string();
        let request = format!(
            "POST /register HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let mut conn = tokio::net::TcpStream::connect(("127.0.0.1", reg_port))
            .await
            .expect("client SHALL connect");
        conn.write_all(request.as_bytes()).await.expect("client SHALL send request");
        let mut response = String::new();
        conn.read_to_string(&mut response).await.expect("client SHALL read response");

        assert!(response.starts_with("HTTP/1.1 200 OK"), "register SHALL 200 even with empty body");
        let got = state.get("unknown").expect("missing device_id SHALL default to 'unknown'");
        assert_eq!(got.platform, "", "missing platform SHALL default to empty");
        assert_eq!(got.device_name, "", "missing device_name SHALL default to empty");
        assert_eq!(got.version, "", "missing version SHALL default to empty");
    }
}
