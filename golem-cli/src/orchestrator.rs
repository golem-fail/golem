//! Single-instance orchestrator for coordinating multiple golem processes.
//!
//! The first `golem run` becomes the server, listening on a unix socket.
//! Subsequent `golem run` calls detect the server and submit work to it
//! instead of starting a new process. This prevents device/companion races
//! and enables shared resource management.
//!
//! Protocol: JSON objects terminated by newline over unix domain socket
//! at `~/.golem/golem.sock`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Path to the orchestrator socket.
fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".golem");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("golem.sock")
}

/// Try to connect to an existing orchestrator server.
///
/// Returns the connected stream if successful, or an error if no server
/// is running (socket doesn't exist or connection refused).
pub async fn try_connect() -> Result<UnixStream> {
    let path = socket_path();
    if !path.exists() {
        anyhow::bail!("no socket at {}", path.display());
    }

    let stream = UnixStream::connect(&path)
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
    tokio::time::timeout(std::time::Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .context("ping timeout")?
        .context("failed to read pong")?;

    if !line.contains("pong") {
        anyhow::bail!("unexpected response to ping: {line}");
    }

    // Reconnect since we consumed the stream in the ping check
    let stream = UnixStream::connect(&path).await?;
    Ok(stream)
}

/// The orchestrator server.
///
/// Listens on a unix socket and handles client connections.
/// Runs in the background via `tokio::spawn`.
pub struct OrchestratorServer {
    _handle: tokio::task::JoinHandle<()>,
}

impl OrchestratorServer {
    /// Clean up the socket file on drop.
    fn cleanup() {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
    }
}

impl Drop for OrchestratorServer {
    fn drop(&mut self) {
        Self::cleanup();
    }
}

/// Start the orchestrator server in the background.
///
/// Creates the unix socket, spawns a task to accept connections and
/// handle messages. Returns the server handle.
pub async fn start_server() -> Result<OrchestratorServer> {
    let path = socket_path();

    // Remove stale socket if it exists
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    let listener = UnixListener::bind(&path)
        .with_context(|| format!("failed to bind socket at {}", path.display()))?;

    eprintln!("  Orchestrator: listening on {}", path.display());

    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(handle_client(stream));
                }
                Err(e) => {
                    eprintln!("  Orchestrator: accept error: {e}");
                    break;
                }
            }
        }
    });

    Ok(OrchestratorServer { _handle: handle })
}

/// Handle a single client connection.
async fn handle_client(stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // client disconnected
            Ok(_) => {
                let response = process_message(&line).await;
                if let Err(e) = writer.write_all(format!("{}\n", response).as_bytes()).await {
                    eprintln!("  Orchestrator: write error: {e}");
                    break;
                }
            }
            Err(e) => {
                eprintln!("  Orchestrator: read error: {e}");
                break;
            }
        }
    }
}

/// Process a single message from a client and return a response.
async fn process_message(msg: &str) -> String {
    let json: serde_json::Value = match serde_json::from_str(msg.trim()) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({"type": "error", "message": format!("invalid JSON: {e}")})
                .to_string();
        }
    };

    match json["type"].as_str() {
        Some("ping") => serde_json::json!({"type": "pong"}).to_string(),
        Some("status") => {
            serde_json::json!({
                "type": "status",
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
            })
            .to_string()
        }
        Some("submit") => {
            // Placeholder — actual work submission in session 2
            serde_json::json!({
                "type": "accepted",
                "job_id": "placeholder",
                "message": "Work submission not yet implemented. Run flows directly for now.",
            })
            .to_string()
        }
        Some(other) => {
            serde_json::json!({"type": "error", "message": format!("unknown message type: {other}")})
                .to_string()
        }
        None => {
            serde_json::json!({"type": "error", "message": "missing 'type' field"})
                .to_string()
        }
    }
}

/// Submit work to a running orchestrator (placeholder for session 2).
///
/// For now, just prints that the connection was successful and returns.
pub async fn submit_and_wait(
    _stream: UnixStream,
    flow_paths: &[PathBuf],
    _config: &serde_json::Value,
) -> Result<()> {
    eprintln!(
        "  Connected to orchestrator. Submitting {} flow(s)...",
        flow_paths.len()
    );
    eprintln!("  Work submission not yet implemented. Run flows directly for now.");
    // Session 2 will implement actual submission and result streaming
    Ok(())
}
