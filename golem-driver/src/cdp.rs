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

/// JavaScript that traverses the DOM and returns a tree matching the Android
/// companion's JSON format (class, text, contentDescription, bounds, children).
/// - Excludes `display: none` and `visibility: hidden` (matches accessibility)
/// - Includes dynamically added elements (fixes the `{#if}` bug)
/// - Uses `getBoundingClientRect()` for viewport-relative coordinates
/// Returns JSON: `{ tree: {...}, meta: { elapsed_ms, node_count, dpr, url } }`
///
/// Text = what the user SEES. accessibility_label = aria-label (for screen readers).
///
/// Text priority:
///   Inputs: value → placeholder → text content → aria-label
///   Others: text content → aria-label
///
/// contentDescription (→ accessibility_label): always aria-label || id
const DOM_TRAVERSAL_JS: &str = r#"(function(){var dpr=window.devicePixelRatio||1;var nc=0;var t0=performance.now();function t(el){nc++;var r=el.getBoundingClientRect();var al=el.getAttribute('aria-label')||'';var ph=el.placeholder||'';var tx='';for(var c of el.childNodes){if(c.nodeType===3&&c.textContent.trim()){tx=c.textContent.trim();break;}}var isInput=el.tagName==='INPUT'||el.tagName==='TEXTAREA'||el.tagName==='SELECT';var val=(isInput&&el.type!=='checkbox'&&el.type!=='radio')?el.value||'':'';var text=val?val:ph?ph:tx?tx:al;var n={class:el.tagName.toLowerCase(),text:text,contentDescription:al||el.id||'',bounds:{left:Math.round(r.left*dpr),top:Math.round(r.top*dpr),right:Math.round((r.left+r.width)*dpr),bottom:Math.round((r.top+r.height)*dpr)},clickable:el.tagName==='BUTTON'||el.tagName==='A'||el.getAttribute('role')==='button',enabled:!el.disabled,checked:!!el.checked,focused:document.activeElement===el,scrollable:false,selected:false,children:[]};for(var c of el.children){if(c.tagName!=='SCRIPT'&&c.tagName!=='STYLE'){var s=window.getComputedStyle(c);if(s.display!=='none'&&s.visibility!=='hidden'){n.children.push(t(c));}}}return n;}var tree=t(document.body);return JSON.stringify({tree:tree,meta:{elapsed_ms:Math.round(performance.now()-t0),node_count:nc,dpr:dpr,url:location.href}});})()
"#;

/// Discover the WebView debug socket name for a device.
/// Returns the socket name (e.g. "webview_devtools_remote_12345") or None.
pub async fn find_webview_socket(device_serial: &str) -> Option<String> {
    let output = tokio::process::Command::new("adb")
        .args(["-s", device_serial, "shell", "cat", "/proc/net/unix"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(idx) = line.find("webview_devtools_remote") {
            let name = line[idx..].trim().split_whitespace().next()?;
            return Some(name.to_string());
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

/// Evaluate the DOM traversal JavaScript via CDP and return the raw JSON string.
/// Uses an existing ADB forward (assumes `setup_forward` was called earlier).
pub async fn evaluate_dom_js_cached(port: u16, page_id: &str) -> Result<String> {
    let url = format!(
        "ws://localhost:{port}/devtools/page/{page_id}"
    );

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
        .await
        .context("CDP WebSocket connection failed")?;

    let (mut write, mut read) = ws_stream.split();

    // Send Runtime.evaluate command
    let cmd = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": DOM_TRAVERSAL_JS.trim(),
            "returnByValue": true
        }
    });

    write
        .send(Message::Text(cmd.to_string().into()))
        .await
        .context("failed to send CDP command")?;

    // Read response
    while let Some(msg) = read.next().await {
        let msg = msg.context("CDP WebSocket read error")?;
        if let Message::Text(text) = msg {
            let resp: serde_json::Value =
                serde_json::from_str(&text).context("failed to parse CDP response")?;

            // Check if this is our response (id: 1)
            if resp.get("id").and_then(|v| v.as_i64()) == Some(1) {
                // Extract result.result.value
                if let Some(value) = resp
                    .get("result")
                    .and_then(|r| r.get("result"))
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_str())
                {
                    return Ok(value.to_string());
                }

                // Check for error
                if let Some(err) = resp.get("result").and_then(|r| r.get("exceptionDetails")) {
                    bail!("CDP evaluation error: {err}");
                }

                bail!("CDP response missing result value");
            }
        }
    }

    bail!("CDP WebSocket closed without response")
}

/// Fetch the live DOM tree from an Android WebView via CDP.
///
/// Returns a JSON tree in the Android companion format (class, text,
/// contentDescription, bounds with left/top/right/bottom, children),
/// with bounds offset by the WebView's screen position.
///
/// Returns None if:
/// - No WebView debug socket found (debugging not enabled)
/// - CDP connection fails
/// - JS evaluation fails
pub async fn fetch_webview_dom(
    device_serial: &str,
    webview_bounds_left: i32,
    webview_bounds_top: i32,
) -> Option<serde_json::Value> {
    let socket_name = find_webview_socket(device_serial).await?;
    let port = setup_forward(device_serial, &socket_name).await.ok()?;

    let result = async {
        let page_id = get_page_id(port).await?;
        let dom_json = evaluate_dom_js_cached(port, &page_id).await?;
        let mut dom: serde_json::Value =
            serde_json::from_str(&dom_json).context("failed to parse CDP DOM JSON")?;

        // Offset bounds by WebView's screen position
        offset_bounds(&mut dom, webview_bounds_left, webview_bounds_top);

        Ok::<_, anyhow::Error>(dom)
    }
    .await;

    // Clean up forward (best-effort)
    let _ = remove_forward(device_serial, port).await;

    result.ok()
}

/// Recursively offset bounds in a CDP DOM tree by the WebView's screen position.
pub fn offset_bounds(node: &mut serde_json::Value, dx: i32, dy: i32) {
    if let Some(bounds) = node.get_mut("bounds").and_then(|b| b.as_object_mut()) {
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
    if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
        for child in children {
            offset_bounds(child, dx, dy);
        }
    }
}
