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
/// Runs in the background via `tokio::spawn`. Shares a ResourceManager
/// AND an InstallCache with the main suite runner so client and server
/// flows coordinate device allocation *and* avoid re-running install
/// scripts on devices where a previous submit already installed. Cache
/// lifetime = server process lifetime; the cache naturally drains when
/// the server exits.
pub struct OrchestratorServer {
    _handle: tokio::task::JoinHandle<()>,
    /// Shared resource manager for all flows (server + client).
    pub resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    /// Shared install cache. All submits — server's own run and every
    /// client submit — see the same `(udid, bundle) → Succeeded` entries,
    /// so a device installed by submit N skips install for submit N+1.
    pub install_cache: golem_runner::installer::InstallCache,
    /// Count of active client handlers. Server waits for this to reach 0 before exiting.
    active_clients: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl OrchestratorServer {
    /// Wait for all active client handlers to complete, then clean up.
    ///
    /// In-process callers (own server + own submit-and-wait) typically
    /// see a count of 1 for ~hundreds of ms while the kernel finalises
    /// the unix-socket peer close. Stay quiet until either the count
    /// stays >1 for a while (real concurrent clients), or `--debug` is
    /// set — the historical "waiting for 1 active client(s)..." noise
    /// on every successful run was just the self-loopback.
    pub async fn wait_for_clients(&self) {
        use std::sync::atomic::Ordering;
        let mut last_logged_count = 0u32;
        let mut ticks_with_count = 0u32;
        loop {
            let count = self.active_clients.load(Ordering::Acquire);
            if count == 0 {
                break;
            }
            // Only emit if (a) genuinely multi-client (>=2) on first
            // observation, or (b) --debug, or (c) count stays >=1 for
            // more than ~3s (kernel close not finalising — actual hang).
            ticks_with_count += 1;
            let noisy_enough = count >= 2 || golem_driver::is_debug() || ticks_with_count > 3;
            if noisy_enough && count != last_logged_count {
                eprintln!("  [orchestrator] waiting for {count} active client(s)...");
                last_logged_count = count;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    /// Clean up the socket file.
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

    eprintln!("  [orchestrator] server — listening on {}", path.display());

    let resource_mgr = std::sync::Arc::new(
        golem_devices::resource_manager::ResourceManager::new(
            golem_devices::concurrency::ConcurrencyConfig::default(),
        ),
    );
    let install_cache = golem_runner::installer::InstallCache::new();

    let active_clients = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let rm = resource_mgr.clone();
    let ic = install_cache.clone();
    let ac = active_clients.clone();
    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let rm = rm.clone();
                    let ic = ic.clone();
                    let ac = ac.clone();
                    ac.fetch_add(1, std::sync::atomic::Ordering::Release);
                    tokio::spawn(async move {
                        handle_client(stream, rm, ic).await;
                        ac.fetch_sub(1, std::sync::atomic::Ordering::Release);
                    });
                }
                Err(e) => {
                    eprintln!("  [orchestrator] accept error: {e}");
                    break;
                }
            }
        }
    });

    Ok(OrchestratorServer { _handle: handle, resource_mgr, install_cache, active_clients })
}

/// Handle a single client connection.
async fn handle_client(
    stream: UnixStream,
    resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: golem_runner::installer::InstallCache,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let writer = std::sync::Arc::new(tokio::sync::Mutex::new(writer));
    let mut line = String::new();

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
                    Some("submit") => {
                        handle_submit(&json, &resource_mgr, &install_cache, &writer).await;
                    }
                    Some(other) => {
                        let resp = serde_json::json!({"type": "error", "message": format!("unknown message type: {other}")});
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
                    }
                    None => {
                        let resp = serde_json::json!({"type": "error", "message": "missing 'type' field"});
                        let mut w = writer.lock().await;
                        let _ = w.write_all(format!("{}\n", resp).as_bytes()).await;
                    }
                }
            }
            Err(e) => {
                // Client disconnects mid-read are normal (especially
                // in-process clients drop the socket as soon as
                // submit_and_wait returns). Stay quiet unless --debug.
                if golem_driver::is_debug() {
                    eprintln!("  [orchestrator] read error: {e}");
                }
                break;
            }
        }
    }
}

/// Handle a "submit" message: run the suite and stream events to the client.
async fn handle_submit(
    json: &serde_json::Value,
    resource_mgr: &std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: &golem_runner::installer::InstallCache,
    writer: &std::sync::Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
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
    let rebuild = cfg["rebuild"].as_bool().unwrap_or(false);
    let no_build = cfg["no_build"].as_bool().unwrap_or(false);
    let record = cfg["record"].as_bool().unwrap_or(false);
    let no_record = cfg["no_record"].as_bool().unwrap_or(false);
    let trace = cfg["trace"].as_bool().unwrap_or(false);
    let repeat = cfg["repeat"].as_u64().map(|n| n.clamp(1, 100) as u32).unwrap_or(1);

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
        keep_devices,
        no_results,
        start,
        vars,
        output_dir,
        project_root,
        project_apps: project_config.apps,
        coverage_override,
        rebuild,
        no_build,
        device_settings: project_config.device_settings,
        record,
        no_record,
        project_record: project_config.options.record,
        trace,
        repeat,
        // Server doesn't do its own human streaming — client handles output.
        stream_human: false,
    };

    let mut runner =
        SuiteRunner::with_resource_manager(config, resource_mgr.clone(), install_cache.clone());
    runner.event_forwarder = Some(fwd_tx);

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
                    &report, &server_output_dir, include_junit,
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
                }
            })
        }
        Err(e) => {
            serde_json::json!({"type": "error", "message": format!("suite failed: {e}")})
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

    // Read streamed events and final result.
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    let mut all_passed = true;

    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .await
            .context("lost connection to orchestrator")?;

        if line.is_empty() {
            anyhow::bail!("orchestrator disconnected unexpectedly");
        }

        let response: serde_json::Value = serde_json::from_str(line.trim())
            .context("invalid JSON from orchestrator")?;

        match response["type"].as_str() {
            Some("event") => {
                // Deserialize and re-emit locally.
                if let Ok(wire) = serde_json::from_value::<golem_events::WireEvent>(
                    response["event"].clone(),
                ) {
                    let event = wire.into_event();
                    local_tx.emit(event.device_id, event.kind);
                }
            }
            Some("done") => {
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
                    let formats = if include_junit { "json, toon, xml" } else { "json, toon" };
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
                anyhow::bail!("Orchestrator error: {msg}");
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
    Ok(SubmitOutcome { report, all_passed })
}

/// Build a `file://` URI from a string path with percent-encoding so
/// spaces and non-ASCII characters don't break OSC 8 hyperlinks.
/// Mirror of `main.rs::file_uri` for a string input — kept duplicate
/// rather than re-extracting because both crates avoid taking on a
/// utility module just for this two-callsite helper.
fn file_uri_str(path: &str) -> String {
    let mut out = String::from("file://");
    for &c in path.as_bytes() {
        let unreserved = c.is_ascii_alphanumeric()
            || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/');
        if unreserved {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}
