//! Chrome DevTools Protocol (CDP) client for Android WebView DOM access.
//!
//! When the Android accessibility tree doesn't reflect dynamic DOM changes
//! (Svelte `{#if}`, React conditional rendering), this module connects to
//! the WebView's CDP endpoint via ADB forwarding and reads the live DOM.
//!
//! Flow:
//! 1. Discover the WebView debug socket via `adb shell cat /proc/net/unix`
//! 2. Forward it to a local TCP port: `adb forward tcp:PORT localabstract:SOCKET`
//! 3. GET /json to discover page targets
//! 4. WebSocket upgrade + `Runtime.evaluate` with DOM traversal JS
//! 5. Returns JSON tree matching our Android companion format

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// Find a free TCP port for ADB forwarding.
fn find_free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// DOM traversal JavaScript evaluated inside the WebView via CDP
/// (Android) or the WebKit Inspector (iOS). The readable source lives
/// in `src/dom_traversal.js`; `build.rs` minifies it via the
/// `minifier` crate and writes the compact form to `OUT_DIR` — the
/// embedded blob below is what we send over the wire.
pub(crate) const DOM_TRAVERSAL_JS: &str =
    include_str!(concat!(env!("OUT_DIR"), "/dom_traversal.min.js"));

/// Discover the WebView debug socket name for the given app package.
/// Multiple apps with WebViews can be running at once (current app, prior
/// builds, companion app). Match by the package's PID so we always connect
/// to the right WebView instead of whichever socket happens to be listed
/// first in `/proc/net/unix`.
pub async fn find_webview_socket(device_serial: &str, package_name: &str) -> Option<String> {
    let pid_out = tokio::process::Command::new("adb")
        .args(["-s", device_serial, "shell", "pidof", package_name])
        .output()
        .await
        .ok()?;
    // `pidof` can return multiple PIDs (main + helper processes) separated
    // by spaces. Collect them all and try each — the WebView socket is only
    // registered by the process that actually hosts the WebView.
    let pid_text = String::from_utf8_lossy(&pid_out.stdout);
    let pids: Vec<&str> = pid_text.split_whitespace().collect();
    if pids.is_empty() {
        return None;
    }

    let output = tokio::process::Command::new("adb")
        .args(["-s", device_serial, "shell", "cat", "/proc/net/unix"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    for pid in &pids {
        let suffix = format!("webview_devtools_remote_{pid}");
        for line in text.lines() {
            if let Some(idx) = line.find(&suffix) {
                let name = line[idx..].split_whitespace().next()?;
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Set up ADB forward from a free local TCP port to the abstract Unix socket.
/// Returns the allocated port.
pub async fn setup_forward(device_serial: &str, socket_name: &str) -> Result<u16> {
    let port = find_free_port().context("failed to find free port for CDP forward")?;

    let output = tokio::process::Command::new("adb")
        .args([
            "-s",
            device_serial,
            "forward",
            &format!("tcp:{port}"),
            &format!("localabstract:{socket_name}"),
        ])
        .output()
        .await
        .context("failed to set up ADB forward for CDP")?;

    if !output.status.success() {
        bail!(
            "ADB forward failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(port)
}

/// Remove all ADB forwards to webview_devtools_remote sockets for this device.
/// Prevents stale forwards from accumulating across test runs.
pub async fn cleanup_stale_forwards(device_serial: &str) {
    let output = tokio::process::Command::new("adb")
        .args(["-s", device_serial, "forward", "--list"])
        .output()
        .await;

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("webview_devtools_remote") {
                if let Some(tcp_part) = line.split_whitespace().nth(1) {
                    let _ = tokio::process::Command::new("adb")
                        .args(["-s", device_serial, "forward", "--remove", tcp_part])
                        .output()
                        .await;
                }
            }
        }
    }
}

/// Remove an ADB forward.
pub async fn remove_forward(device_serial: &str, port: u16) -> Result<()> {
    let _ = tokio::process::Command::new("adb")
        .args([
            "-s",
            device_serial,
            "forward",
            "--remove",
            &format!("tcp:{port}"),
        ])
        .output()
        .await;
    Ok(())
}

/// Get the page target ID from the CDP /json endpoint.
pub async fn get_page_id(port: u16) -> Result<String> {
    let url = format!("http://localhost:{port}/json");
    let resp = reqwest::get(&url)
        .await
        .context("CDP /json request failed")?
        .text()
        .await?;

    let targets: serde_json::Value =
        serde_json::from_str(&resp).context("failed to parse CDP /json")?;

    let arr = targets.as_array().context("CDP /json not an array")?;
    if arr.is_empty() {
        bail!("No CDP page targets found");
    }

    arr[0]
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .context("CDP page target has no id")
}

/// Evaluate a JavaScript expression in the WebView via CDP.
///
/// Pass `await_promise = true` for async expressions (the DOM traversal
/// uses this); `false` for fire-and-forget hooks like
/// `__golemSetLocation`. Returns the result as a string when it can
/// be coerced; for `undefined`/`null` returns an empty string.
pub async fn evaluate_js(
    port: u16,
    page_id: &str,
    expression: &str,
    await_promise: bool,
) -> Result<String> {
    let url = format!("ws://localhost:{port}/devtools/page/{page_id}");

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
        .await
        .context("CDP WebSocket connection failed")?;

    let (mut write, mut read) = ws_stream.split();

    let cmd = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": await_promise
        }
    });

    write
        .send(Message::Text(cmd.to_string().into()))
        .await
        .context("failed to send CDP command")?;

    while let Some(msg) = read.next().await {
        let msg = msg.context("CDP WebSocket read error")?;
        if let Message::Text(text) = msg {
            let resp: serde_json::Value =
                serde_json::from_str(&text).context("failed to parse CDP response")?;

            if resp.get("id").and_then(|v| v.as_i64()) == Some(1) {
                if let Some(err) = resp.get("result").and_then(|r| r.get("exceptionDetails")) {
                    bail!("CDP evaluation error: {err}");
                }
                let value = resp
                    .get("result")
                    .and_then(|r| r.get("result"))
                    .and_then(|r| r.get("value"));
                return Ok(match value {
                    Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
                    Some(v) if v.is_null() => String::new(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                });
            }
        }
    }

    bail!("CDP WebSocket closed without response")
}

/// Evaluate the DOM traversal JavaScript via CDP and return the raw JSON string.
/// Uses an existing ADB forward (assumes `setup_forward` was called earlier).
pub async fn evaluate_dom_js_cached(port: u16, page_id: &str) -> Result<String> {
    evaluate_js(port, page_id, DOM_TRAVERSAL_JS.trim(), true).await
}

/// Recursively scale all coordinate values in bounds/visible_bounds by a factor.
/// Converts from CSS pixels (as reported by JS) to device pixels (Android coordinate system).
pub fn scale_bounds_by_dpr(node: &mut serde_json::Value, dpr: f64) {
    for key in &["bounds", "visible_bounds"] {
        if let Some(bounds) = node.get_mut(*key).and_then(|b| b.as_object_mut()) {
            for field in &["left", "top", "right", "bottom"] {
                if let Some(v) = bounds.get(*field).and_then(|v| v.as_i64()) {
                    bounds.insert(
                        field.to_string(),
                        serde_json::json!((v as f64 * dpr).round() as i32),
                    );
                }
            }
        }
    }
    if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
        for child in children {
            scale_bounds_by_dpr(child, dpr);
        }
    }
}

/// Recursively offset bounds in a CDP DOM tree by the WebView's screen position.
pub fn offset_bounds(node: &mut serde_json::Value, dx: i32, dy: i32) {
    // Offset both `bounds` and `visible_bounds`
    for key in &["bounds", "visible_bounds"] {
        if let Some(bounds) = node.get_mut(*key).and_then(|b| b.as_object_mut()) {
            if let Some(v) = bounds.get("left").and_then(|v| v.as_i64()) {
                bounds.insert("left".to_string(), serde_json::json!(v as i32 + dx));
            }
            if let Some(v) = bounds.get("top").and_then(|v| v.as_i64()) {
                bounds.insert("top".to_string(), serde_json::json!(v as i32 + dy));
            }
            if let Some(v) = bounds.get("right").and_then(|v| v.as_i64()) {
                bounds.insert("right".to_string(), serde_json::json!(v as i32 + dx));
            }
            if let Some(v) = bounds.get("bottom").and_then(|v| v.as_i64()) {
                bounds.insert("bottom".to_string(), serde_json::json!(v as i32 + dy));
            }
        }
    }
    if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
        for child in children {
            offset_bounds(child, dx, dy);
        }
    }
}
