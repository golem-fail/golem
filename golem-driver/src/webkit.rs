//! WebKit Inspector Protocol client for iOS WebView DOM access (simulator).
//!
//! Connects to the iOS simulator's WebKit Inspector service via Unix domain
//! socket, performs the binary plist RPC handshake, and evaluates JavaScript
//! in WKWebView pages — the same `IntersectionObserver` DOM traversal used
//! by the Android CDP path.
//!
//! Flow:
//! 1. Discover the inspector socket via glob on `/private/tmp/com.apple.launchd.*`
//! 2. Connect and perform the RPC handshake (reportIdentifier → listing → socketSetup)
//! 3. Send `Runtime.evaluate` with DOM traversal JS via `_rpc_forwardSocketData:`
//! 4. Returns JSON tree matching the companion format (with visible_bounds)
//!
//! Physical device support (usbmuxd + lockdown TLS) is a future extension —
//! the transport is behind a trait to allow swapping the underlying stream.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

// ---------------------------------------------------------------------------
// Socket discovery
// ---------------------------------------------------------------------------

/// Find WebKit Inspector Unix domain socket candidates for the iOS simulator.
///
/// Many stale sockets accumulate in `/private/tmp/com.apple.launchd.*/` from
/// old simulator sessions. Returns all candidates — the caller should try
/// connecting to each one.
pub(crate) async fn find_inspector_sockets() -> Vec<PathBuf> {
    let base = PathBuf::from("/private/tmp");
    let mut entries = match tokio::fs::read_dir(&base).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut candidates = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("com.apple.launchd.") {
            let socket_path = entry.path().join("com.apple.webinspectord_sim.socket");
            if tokio::fs::metadata(&socket_path).await.is_ok() {
                candidates.push(socket_path);
            }
        }
    }
    candidates
}

// ---------------------------------------------------------------------------
// Transport: framed binary plist over Unix socket
// ---------------------------------------------------------------------------

/// Simulator transport: 4-byte big-endian length-prefixed binary plist
/// messages over a Unix domain socket.
pub(crate) struct SimulatorTransport {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    /// Buffer for partial message reassembly (WIRPartialMessageKey).
    partial_buf: Vec<u8>,
}

impl SimulatorTransport {
    /// Connect to the inspector socket.
    pub(crate) async fn connect(path: &std::path::Path) -> Result<Self> {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .with_context(|| format!("failed to connect to {}", path.display()))?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader,
            writer,
            partial_buf: Vec::new(),
        })
    }

    /// Send a plist value as a length-prefixed binary plist message.
    async fn send_plist(&mut self, msg: &plist::Value) -> Result<()> {
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, msg)
            .context("failed to serialize plist")?;
        let len = buf.len() as u32;
        self.writer.write_all(&len.to_be_bytes()).await?;
        self.writer.write_all(&buf).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Receive a complete plist message, handling partial message reassembly.
    ///
    /// The WebKit Inspector protocol splits large messages using
    /// `WIRPartialMessageKey` (intermediate chunks) and `WIRFinalMessageKey`
    /// (last chunk). We accumulate partials and return the reassembled plist.
    async fn recv_plist(&mut self) -> Result<plist::Value> {
        loop {
            let msg = self.recv_raw_plist().await?;

            // Check for partial/final message keys
            if let Some(dict) = msg.as_dictionary() {
                if let Some(partial) = dict.get("WIRPartialMessageKey") {
                    if let Some(data) = partial.as_data() {
                        self.partial_buf.extend_from_slice(data);
                        continue; // Wait for more chunks
                    }
                }
                if let Some(final_part) = dict.get("WIRFinalMessageKey") {
                    if let Some(data) = final_part.as_data() {
                        self.partial_buf.extend_from_slice(data);
                        let reassembled: plist::Value =
                            plist::from_bytes(&self.partial_buf)
                                .context("failed to parse reassembled plist")?;
                        self.partial_buf.clear();
                        return Ok(reassembled);
                    }
                }
            }

            // Not a partial message — return directly
            if !self.partial_buf.is_empty() {
                // Shouldn't happen: got a non-partial after partials without a final.
                // Clear buffer and return what we got.
                self.partial_buf.clear();
            }
            return Ok(msg);
        }
    }

    /// Read a single length-prefixed plist frame from the socket.
    async fn recv_raw_plist(&mut self) -> Result<plist::Value> {
        let mut len_buf = [0u8; 4];
        self.reader
            .read_exact(&mut len_buf)
            .await
            .context("failed to read plist length header")?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len == 0 {
            bail!("WebKit Inspector: received zero-length message");
        }
        if len > 64 * 1024 * 1024 {
            bail!("WebKit Inspector message too large: {len} bytes");
        }

        let mut buf = vec![0u8; len];
        self.reader
            .read_exact(&mut buf)
            .await
            .with_context(|| format!("failed to read plist body ({len} bytes)"))?;

        plist::from_bytes(&buf)
            .with_context(|| format!("failed to deserialize plist ({len} bytes)"))
    }
}

// ---------------------------------------------------------------------------
// RPC helpers
// ---------------------------------------------------------------------------

/// Build an RPC message dictionary: `{ "__selector": selector, "__argument": args }`.
fn build_rpc(selector: &str, args: plist::Dictionary) -> plist::Value {
    let mut msg = plist::Dictionary::new();
    msg.insert(
        "__selector".to_string(),
        plist::Value::String(selector.to_string()),
    );
    msg.insert("__argument".to_string(), plist::Value::Dictionary(args));
    plist::Value::Dictionary(msg)
}

/// Extract selector and argument from an incoming RPC message.
fn parse_rpc(msg: &plist::Value) -> Option<(&str, &plist::Dictionary)> {
    let dict = msg.as_dictionary()?;
    let selector = dict.get("__selector")?.as_string()?;
    let args = dict.get("__argument")?.as_dictionary()?;
    Some((selector, args))
}

/// Extract JSON message data from an `_rpc_applicationSentData:` message.
/// Only returns data destined for our sender_id.
fn extract_message_data(msg: &plist::Value, sender_id: &str) -> Option<String> {
    let (selector, args) = parse_rpc(msg)?;
    if selector != "_rpc_applicationSentData:" {
        return None;
    }
    // Check if this message is for us
    let dest = args.get("WIRDestinationKey")?.as_string()?;
    if dest != sender_id {
        return None;
    }
    // Extract the JSON data
    let data = args.get("WIRMessageDataKey")?;
    match data {
        plist::Value::Data(bytes) => String::from_utf8(bytes.clone()).ok(),
        plist::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// WebKit Inspector client
// ---------------------------------------------------------------------------

/// High-level WebKit Inspector client. Manages the RPC handshake and
/// provides `evaluate_js()` for running JavaScript in a WKWebView page.
pub(crate) struct WebKitInspector {
    transport: SimulatorTransport,
    connection_id: String,
    app_id: String,
    page_id: u64,
    sender_id: String,
    /// Target ID from Target.targetCreated (iOS 12.2+ protocol).
    target_id: Option<String>,
    next_cmd_id: u32,
}

impl WebKitInspector {
    /// Discover the simulator inspector socket, connect, and complete the
    /// full handshake. Tries each candidate socket until one works.
    pub(crate) async fn connect() -> Result<Self> {
        let candidates = find_inspector_sockets().await;
        if candidates.is_empty() {
            bail!("no WebKit Inspector socket found — is a simulator running?");
        }

        let mut last_err = None;
        for socket_path in &candidates {
            let transport = match SimulatorTransport::connect(socket_path).await {
                Ok(t) => t,
                Err(_) => continue, // Stale socket — try next
            };
            let connection_id = uuid::Uuid::new_v4().to_string().to_uppercase();
            let sender_id = uuid::Uuid::new_v4().to_string().to_uppercase();

            let mut inspector = Self {
                transport,
                connection_id,
                app_id: String::new(),
                page_id: 0,
                sender_id,
                target_id: None,
                next_cmd_id: 1,
            };

            match inspector.handshake().await {
                Ok(()) => return Ok(inspector),
                Err(e) => {
                    eprintln!("  [webkit] handshake failed on {}: {e}", socket_path.display());
                    last_err = Some(e);
                    continue;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("all {} inspector sockets failed to connect", candidates.len())
        }))
    }

    /// Perform the full RPC handshake:
    /// 1. reportIdentifier → reportCurrentState
    /// 2. Receive reportConnectedApplicationList, applicationConnected, applicationSentListing
    /// 3. forwardSocketSetup (open inspector channel for the page)
    ///
    /// The server sends messages in a specific order after reportIdentifier:
    ///   _rpc_reportCurrentState:
    ///   _rpc_reportConnectedApplicationList:
    ///   _rpc_applicationConnected: (one per app, may be several)
    ///   _rpc_applicationSentListing: (page list for each app)
    ///   _rpc_applicationUpdated: (app becomes ready)
    async fn handshake(&mut self) -> Result<()> {
        // Step 1: Report our identity
        let mut args = plist::Dictionary::new();
        args.insert(
            "WIRConnectionIdentifierKey".to_string(),
            plist::Value::String(self.connection_id.clone()),
        );
        self.transport
            .send_plist(&build_rpc("_rpc_reportIdentifier:", args))
            .await?;

        // Step 2: Receive messages until we find a page to inspect.
        let mut pages_received = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);

        while tokio::time::Instant::now() < deadline {
            let msg = match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.transport.recv_plist(),
            )
            .await
            {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => bail!("handshake recv failed: {e:#}"),
                Err(_) => {
                    if pages_received {
                        break; // Timeout is OK after we got pages
                    }
                    bail!("timeout (5s) waiting for WebKit Inspector handshake");
                }
            };

            if let Some((selector, rpc_args)) = parse_rpc(&msg) {
                match selector {
                    "_rpc_reportCurrentState:" | "_rpc_reportConnectedApplicationList:"
                    | "_rpc_reportConnectedDriverList:" => {
                        // Acknowledged — continue receiving
                    }
                    "_rpc_applicationConnected:" | "_rpc_applicationUpdated:" => {
                        // Track app IDs — prefer non-WebContent processes
                        if let Some(app_id) =
                            rpc_args.get("WIRApplicationIdentifierKey").and_then(|v| v.as_string())
                        {
                            let bundle = rpc_args
                                .get("WIRApplicationBundleIdentifierKey")
                                .and_then(|v| v.as_string())
                                .unwrap_or("");
                            // Skip WebContent helper processes
                            if !bundle.contains("WebKit.WebContent") {
                                self.app_id = app_id.to_string();
                            }
                        }
                    }
                    "_rpc_applicationSentListing:" => {
                        // Extract the app this listing belongs to
                        let listing_app = rpc_args
                            .get("WIRApplicationIdentifierKey")
                            .and_then(|v| v.as_string())
                            .unwrap_or("");

                        if let Some(listing) =
                            rpc_args.get("WIRListingKey").and_then(|v| v.as_dictionary())
                        {
                            for (page_key, page_val) in listing {
                                if let Some(page_dict) = page_val.as_dictionary() {
                                    let page_type = page_dict
                                        .get("WIRTypeKey")
                                        .and_then(|v| v.as_string())
                                        .unwrap_or("");
                                    if page_type == "WIRTypeServiceWorker" {
                                        continue;
                                    }
                                    let _url = page_dict
                                        .get("WIRURLKey")
                                        .and_then(|v| v.as_string())
                                        .unwrap_or("");
                                    // Use WIRPageIdentifierKey (integer) if available,
                                    // fall back to parsing the dict key
                                    let pid = page_dict
                                        .get("WIRPageIdentifierKey")
                                        .and_then(|v| v.as_unsigned_integer())
                                        .or_else(|| page_key.parse::<u64>().ok());
                                    if let Some(pid) = pid {
                                        self.page_id = pid;
                                        self.app_id = listing_app.to_string();
                                        pages_received = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    "_rpc_applicationDisconnected:" => {
                        bail!("application disconnected during handshake");
                    }
                    _ => {}
                }
            }

            if pages_received {
                // Drain remaining unsolicited messages without blocking
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    self.transport.recv_plist(),
                )
                .await;
                break;
            }
        }

        if self.app_id.is_empty() {
            bail!("no inspectable application found — does the WKWebView have isInspectable = true?");
        }
        if !pages_received {
            bail!("no inspectable pages found in application {}", self.app_id);
        }

        // Step 3: Open inspector channel for the selected page
        let mut setup_args = plist::Dictionary::new();
        setup_args.insert(
            "WIRConnectionIdentifierKey".to_string(),
            plist::Value::String(self.connection_id.clone()),
        );
        setup_args.insert(
            "WIRApplicationIdentifierKey".to_string(),
            plist::Value::String(self.app_id.clone()),
        );
        setup_args.insert(
            "WIRPageIdentifierKey".to_string(),
            plist::Value::Integer(self.page_id.into()),
        );
        setup_args.insert(
            "WIRSenderKey".to_string(),
            plist::Value::String(self.sender_id.clone()),
        );
        setup_args.insert(
            "WIRAutomaticallyPause".to_string(),
            plist::Value::Boolean(false),
        );
        self.transport
            .send_plist(&build_rpc("_rpc_forwardSocketSetup:", setup_args))
            .await?;

        // Step 4: Wait for Target.targetCreated event (iOS 12.2+ protocol).
        // After forwardSocketSetup, the server sends _rpc_applicationSentData:
        // messages containing Target.targetCreated with the targetId we need.
        let target_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < target_deadline {
            let msg = match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                self.transport.recv_plist(),
            )
            .await
            {
                Ok(Ok(msg)) => msg,
                Ok(Err(_)) | Err(_) => break,
            };

            if let Some(json_data) = extract_message_data(&msg, &self.sender_id) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_data) {
                    if parsed.get("method").and_then(|v| v.as_str()) == Some("Target.targetCreated")
                    {
                        if let Some(target_id) = parsed
                            .get("params")
                            .and_then(|p| p.get("targetInfo"))
                            .and_then(|t| t.get("targetId"))
                            .and_then(|v| v.as_str())
                        {
                            self.target_id = Some(target_id.to_string());
                            let _ = target_id; // used for matching
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Send a JSON inspector command via `_rpc_forwardSocketData:`.
    /// If we have a Target ID (iOS 12.2+), wraps the command in
    /// `Target.sendMessageToTarget`.
    async fn send_inspector_cmd(&mut self, cmd: &serde_json::Value) -> Result<()> {
        let outer = if let Some(ref target_id) = self.target_id {
            // iOS 12.2+ requires wrapping in Target.sendMessageToTarget
            let wrapper_id = self.next_cmd_id;
            self.next_cmd_id += 1;
            serde_json::json!({
                "id": wrapper_id,
                "method": "Target.sendMessageToTarget",
                "params": {
                    "targetId": target_id,
                    "message": cmd.to_string()
                }
            })
        } else {
            cmd.clone()
        };

        let cmd_str = outer.to_string();
        let mut args = plist::Dictionary::new();
        args.insert(
            "WIRConnectionIdentifierKey".to_string(),
            plist::Value::String(self.connection_id.clone()),
        );
        args.insert(
            "WIRApplicationIdentifierKey".to_string(),
            plist::Value::String(self.app_id.clone()),
        );
        args.insert(
            "WIRPageIdentifierKey".to_string(),
            plist::Value::Integer(self.page_id.into()),
        );
        args.insert(
            "WIRSenderKey".to_string(),
            plist::Value::String(self.sender_id.clone()),
        );
        args.insert(
            "WIRSocketDataKey".to_string(),
            plist::Value::Data(cmd_str.into_bytes()),
        );
        self.transport
            .send_plist(&build_rpc("_rpc_forwardSocketData:", args))
            .await
    }

    /// Wait for a Target.dispatchMessageFromTarget response matching the given cmd_id.
    async fn recv_inspector_response(&mut self, cmd_id: u32) -> Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let msg = match tokio::time::timeout(
                std::time::Duration::from_secs(15),
                self.transport.recv_plist(),
            )
            .await
            {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => bail!("recv failed: {e:#}"),
                Err(_) => bail!("timeout (15s) waiting for inspector response"),
            };

            let json_str = match extract_message_data(&msg, &self.sender_id) {
                Some(s) => s,
                None => continue,
            };

            let resp: serde_json::Value = match serde_json::from_str(&json_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Unwrap Target.dispatchMessageFromTarget
            let inner = if resp.get("method").and_then(|v| v.as_str())
                == Some("Target.dispatchMessageFromTarget")
            {
                let msg_str = resp
                    .get("params")
                    .and_then(|p| p.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                match serde_json::from_str::<serde_json::Value>(msg_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                }
            } else {
                resp
            };

            if inner.get("id").and_then(|v| v.as_u64()) == Some(cmd_id as u64) {
                return Ok(inner);
            }
        }
        bail!("timeout waiting for inspector response (cmd_id={cmd_id})")
    }

    /// Evaluate JavaScript in the connected WKWebView page.
    /// Returns the string result.
    ///
    /// WebKit Inspector doesn't support `awaitPromise` on `Runtime.evaluate`
    /// the way CDP does. For async expressions (Promises), we use a two-step
    /// approach: evaluate to get the Promise objectId, then `Runtime.awaitPromise`.
    pub(crate) async fn evaluate_js(&mut self, expression: &str) -> Result<String> {
        // Step 1: Evaluate expression (without returnByValue to get objectId for Promises)
        let eval_id = self.next_cmd_id;
        self.next_cmd_id += 1;
        let eval_cmd = serde_json::json!({
            "id": eval_id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "returnByValue": false
            }
        });
        self.send_inspector_cmd(&eval_cmd).await?;
        let eval_resp = self.recv_inspector_response(eval_id).await?;

        // Check for exception
        if let Some(err) = eval_resp.get("result").and_then(|r| r.get("exceptionDetails")) {
            bail!("WebKit JS evaluation error: {err}");
        }

        let result_obj = eval_resp
            .get("result")
            .and_then(|r| r.get("result"))
            .context("missing result in evaluation response")?;

        let result_type = result_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // If the result is a string, return it directly
        if result_type == "string" {
            if let Some(value) = result_obj.get("value").and_then(|v| v.as_str()) {
                return Ok(value.to_string());
            }
        }

        // If the result is a Promise, use Runtime.awaitPromise
        if result_obj.get("className").and_then(|v| v.as_str()) == Some("Promise") {
            let obj_id = result_obj
                .get("objectId")
                .and_then(|v| v.as_str())
                .context("Promise missing objectId")?;

            let await_id = self.next_cmd_id;
            self.next_cmd_id += 1;
            let await_cmd = serde_json::json!({
                "id": await_id,
                "method": "Runtime.awaitPromise",
                "params": {
                    "promiseObjectId": obj_id,
                    "returnByValue": true
                }
            });
            self.send_inspector_cmd(&await_cmd).await?;
            let await_resp = self.recv_inspector_response(await_id).await?;

            if let Some(err) = await_resp.get("result").and_then(|r| r.get("exceptionDetails")) {
                bail!("WebKit JS await error: {err}");
            }

            if let Some(value) = await_resp
                .get("result")
                .and_then(|r| r.get("result"))
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
            {
                return Ok(value.to_string());
            }

            bail!(
                "WebKit awaitPromise missing string value: {}",
                serde_json::to_string_pretty(&await_resp).unwrap_or_default()
            );
        }

        // For other types, try to get value directly
        if let Some(value) = result_obj.get("value").and_then(|v| v.as_str()) {
            return Ok(value.to_string());
        }

        bail!(
            "WebKit unexpected result type '{}': {}",
            result_type,
            serde_json::to_string_pretty(&eval_resp).unwrap_or_default()
        )
    }
}

// ---------------------------------------------------------------------------
// Public API: fetch WebView DOM
// ---------------------------------------------------------------------------

/// Fetch the live DOM tree from an iOS WKWebView via WebKit Inspector.
///
/// Connects to the simulator's inspector socket, evaluates the DOM traversal
/// JavaScript (with IntersectionObserver for visible_bounds), and returns a
/// JSON tree matching the companion format — offset by the WebView's screen
/// position.
///
/// Returns `None` if:
/// - No inspector socket found (simulator not running)
/// - No inspectable WKWebView (isInspectable not set)
/// - JS evaluation fails
pub(crate) async fn fetch_webview_dom(
    inspector: &mut WebKitInspector,
    webview_bounds_left: i32,
    webview_bounds_top: i32,
) -> Option<serde_json::Value> {
    let dom_json = match inspector
        .evaluate_js(crate::cdp::DOM_TRAVERSAL_JS.trim())
        .await
    {
        Ok(json) => json,
        Err(e) => {
            eprintln!("  [webkit] JS evaluation failed: {e}");
            return None;
        }
    };

    let wrapper: serde_json::Value = match serde_json::from_str(&dom_json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [webkit] failed to parse DOM JSON: {e}");
            return None;
        }
    };

    if let Some(meta) = wrapper.get("meta") {
        let url = meta.get("url").and_then(|v| v.as_str()).unwrap_or("");
        // Skip if page hasn't loaded yet
        if url == "about:blank" || url.is_empty() {
            return None;
        }
    }

    // iOS visual zoom: when an input is focused, WKWebView zooms in.
    // getBoundingClientRect returns CSS pixels relative to the visual viewport.
    // Native accessibility tree uses screen points. The transform:
    //   screen = bcr * visualViewport.scale + webview_screen_offset
    // When not zoomed (scale=1), this simplifies to bcr + offset.
    let vv_scale = wrapper
        .get("meta")
        .and_then(|m| m.get("visualViewport"))
        .and_then(|v| v.get("scale"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);

    if let Some(mut tree) = wrapper.get("tree").cloned() {
        if (vv_scale - 1.0).abs() > 0.01 {
            crate::cdp::scale_bounds_by_dpr(&mut tree, vv_scale);
        }
        crate::cdp::offset_bounds(&mut tree, webview_bounds_left, webview_bounds_top);
        Some(tree)
    } else {
        None
    }
}
