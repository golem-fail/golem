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

use crate::suite::{SuiteConfig, SuiteRunner};

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
            // Extract flow paths and config from the message
            let paths: Vec<PathBuf> = json["flow_paths"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(PathBuf::from))
                        .collect()
                })
                .unwrap_or_default();

            if paths.is_empty() {
                return serde_json::json!({"type": "error", "message": "no flow_paths provided"})
                    .to_string();
            }

            let platform_override = json["config"]["platform"]
                .as_str()
                .and_then(|p| match p {
                    "ios" => Some(golem_devices::Platform::Ios),
                    "android" => Some(golem_devices::Platform::Android),
                    _ => None,
                });

            let seed = json["config"]["seed"].as_u64();

            let config = SuiteConfig {
                platform: platform_override,
                seed,
                ..SuiteConfig::default()
            };

            // Run the suite
            let runner = SuiteRunner::new(config);
            match runner.run_suite(&paths).await {
                Ok(report) => {
                    serde_json::json!({
                        "type": "result",
                        "report": {
                            "total_duration_ms": report.total_duration_ms,
                            "flows": report.flows.iter().map(|f| {
                                serde_json::json!({
                                    "flow_name": f.flow_name,
                                    "success": f.success,
                                    "warnings": f.warnings,
                                    "duration_ms": f.duration_ms,
                                    "device_name": f.device_name,
                                })
                            }).collect::<Vec<_>>(),
                        }
                    })
                    .to_string()
                }
                Err(e) => {
                    serde_json::json!({"type": "error", "message": format!("suite failed: {e}")})
                        .to_string()
                }
            }
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

/// Submit work to a running orchestrator and wait for results.
///
/// Sends the flow paths and config to the server, waits for the result,
/// prints the report, and returns the exit code.
pub async fn submit_and_wait(
    mut stream: UnixStream,
    flow_paths: &[PathBuf],
    config: &serde_json::Value,
) -> Result<bool> {
    eprintln!(
        "  Connected to orchestrator. Submitting {} flow(s)...",
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

    // Read response (may take a long time for suite execution)
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("failed to read result from orchestrator")?;

    let response: serde_json::Value = serde_json::from_str(line.trim())
        .context("invalid JSON response from orchestrator")?;

    match response["type"].as_str() {
        Some("result") => {
            let report = &response["report"];
            let mut all_passed = true;

            if let Some(flows) = report["flows"].as_array() {
                for flow in flows {
                    let name = flow["flow_name"].as_str().unwrap_or("unknown");
                    let success = flow["success"].as_bool().unwrap_or(false);
                    let duration = flow["duration_ms"].as_u64().unwrap_or(0);
                    let device = flow["device_name"].as_str().unwrap_or("");

                    let icon = if success { "✓" } else { "✗" };
                    let status = if success { "PASSED" } else { "FAILED" };
                    let duration_s = duration as f64 / 1000.0;

                    if !device.is_empty() {
                        eprintln!("{icon} {status}  {name} [{device}]  [{duration_s:.1}s]");
                    } else {
                        eprintln!("{icon} {status}  {name}  [{duration_s:.1}s]");
                    }

                    if !success {
                        all_passed = false;
                    }
                }
            }

            let total_ms = report["total_duration_ms"].as_u64().unwrap_or(0);
            let total_s = total_ms as f64 / 1000.0;
            eprintln!("\nTotal: [{total_s:.1}s]");

            Ok(all_passed)
        }
        Some("error") => {
            let msg = response["message"].as_str().unwrap_or("unknown error");
            anyhow::bail!("Orchestrator error: {msg}");
        }
        _ => {
            anyhow::bail!("Unexpected response from orchestrator: {line}");
        }
    }
}
