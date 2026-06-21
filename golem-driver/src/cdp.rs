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

use anyhow::{Context, Result};
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
    match_webview_socket(&text, &pids)
}

/// Pure socket-matching logic for `find_webview_socket`: scan the contents of
/// `/proc/net/unix` for a `webview_devtools_remote_<pid>` socket name, trying
/// each PID in order. Returns the first matching socket name.
fn match_webview_socket(proc_net_unix: &str, pids: &[&str]) -> Option<String> {
    for pid in pids {
        let suffix = format!("webview_devtools_remote_{pid}");
        for line in proc_net_unix.lines() {
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
        return Err(golem_events::coded(
            golem_events::FailureCode::DeviceDriverOpFailed,
            anyhow::anyhow!(
                "ADB forward failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
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

    parse_page_id(&resp)
}

/// Pure target-selection logic for `get_page_id`: parse the CDP `/json`
/// response body and return the `id` of the first page target.
fn parse_page_id(resp: &str) -> Result<String> {
    let targets: serde_json::Value =
        serde_json::from_str(resp).context("failed to parse CDP /json")?;

    let arr = targets.as_array().context("CDP /json not an array")?;
    if arr.is_empty() {
        return Err(golem_events::coded(
            golem_events::FailureCode::DeviceWebviewComms,
            anyhow::anyhow!("No CDP page targets found"),
        ));
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
            if let Some(result) = parse_eval_response(&text)? {
                return Ok(result);
            }
        }
    }

    Err(golem_events::coded(
        golem_events::FailureCode::DeviceWebviewComms,
        anyhow::anyhow!("CDP WebSocket closed without response"),
    ))
}

/// Pure response coercion for `evaluate_js`: parse a single CDP WebSocket text
/// frame. Returns:
/// - `Ok(Some(value))` for the matching `id == 1` response, coerced to a string
///   (`undefined`/`null` → empty string, strings unquoted, others stringified).
/// - `Ok(None)` for any other message — the caller keeps reading.
/// - `Err(..)` if the frame is unparseable or carries `exceptionDetails`.
fn parse_eval_response(text: &str) -> Result<Option<String>> {
    let resp: serde_json::Value =
        serde_json::from_str(text).context("failed to parse CDP response")?;

    if resp.get("id").and_then(|v| v.as_i64()) != Some(1) {
        return Ok(None);
    }

    if let Some(err) = resp.get("result").and_then(|r| r.get("exceptionDetails")) {
        return Err(golem_events::coded(
            golem_events::FailureCode::DeviceWebviewComms,
            anyhow::anyhow!("CDP evaluation error: {err}"),
        ));
    }
    let value = resp
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get("value"));
    Ok(Some(match value {
        Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
        Some(v) if v.is_null() => String::new(),
        Some(v) => v.to_string(),
        None => String::new(),
    }))
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
    // hit_points carry x/y in CSS px — scale them the same way.
    if let Some(points) = node.get_mut("hit_points").and_then(|p| p.as_array_mut()) {
        for pt in points.iter_mut().filter_map(|p| p.as_object_mut()) {
            for field in &["x", "y"] {
                if let Some(v) = pt.get(*field).and_then(|v| v.as_i64()) {
                    pt.insert(
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
    // hit_points carry x/y screen coords — offset them by the WebView origin.
    if let Some(points) = node.get_mut("hit_points").and_then(|p| p.as_array_mut()) {
        for pt in points.iter_mut().filter_map(|p| p.as_object_mut()) {
            if let Some(v) = pt.get("x").and_then(|v| v.as_i64()) {
                pt.insert("x".to_string(), serde_json::json!(v as i32 + dx));
            }
            if let Some(v) = pt.get("y").and_then(|v| v.as_i64()) {
                pt.insert("y".to_string(), serde_json::json!(v as i32 + dy));
            }
        }
    }
    if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
        for child in children {
            offset_bounds(child, dx, dy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 1. scale_bounds_by_dpr scales all four edges of `bounds` by the factor.
    #[test]
    fn scale_scales_all_bounds_edges() {
        let mut node = json!({
            "bounds": { "left": 10, "top": 20, "right": 30, "bottom": 40 }
        });
        scale_bounds_by_dpr(&mut node, 2.0);
        let b = &node["bounds"];
        assert_eq!(b["left"], 20, "left SHALL be scaled by dpr");
        assert_eq!(b["top"], 40, "top SHALL be scaled by dpr");
        assert_eq!(b["right"], 60, "right SHALL be scaled by dpr");
        assert_eq!(b["bottom"], 80, "bottom SHALL be scaled by dpr");
    }

    // 2. scale_bounds_by_dpr also scales `visible_bounds`, not just `bounds`.
    #[test]
    fn scale_scales_visible_bounds_too() {
        let mut node = json!({
            "bounds": { "left": 1, "top": 2, "right": 3, "bottom": 4 },
            "visible_bounds": { "left": 5, "top": 6, "right": 7, "bottom": 8 }
        });
        scale_bounds_by_dpr(&mut node, 3.0);
        assert_eq!(node["bounds"]["left"], 3, "bounds SHALL be scaled");
        assert_eq!(
            node["visible_bounds"]["bottom"], 24,
            "visible_bounds SHALL be scaled"
        );
    }

    // 3. scale_bounds_by_dpr rounds to nearest integer (banker-free round()).
    #[test]
    fn scale_rounds_fractional_results() {
        let mut node = json!({
            "bounds": { "left": 3, "top": 5, "right": 10, "bottom": 1 }
        });
        // 1.5 dpr: 3->4.5->5(round), 5->7.5->8, 10->15, 1->1.5->2
        scale_bounds_by_dpr(&mut node, 1.5);
        let b = &node["bounds"];
        assert_eq!(b["left"], 5, "4.5 SHALL round to 5");
        assert_eq!(b["top"], 8, "7.5 SHALL round to 8");
        assert_eq!(b["right"], 15, "15.0 SHALL stay 15");
        assert_eq!(b["bottom"], 2, "1.5 SHALL round to 2");
    }

    // 4. scale_bounds_by_dpr recurses into children.
    #[test]
    fn scale_recurses_into_children() {
        let mut node = json!({
            "bounds": { "left": 10, "top": 10, "right": 10, "bottom": 10 },
            "children": [
                { "bounds": { "left": 5, "top": 5, "right": 5, "bottom": 5 } }
            ]
        });
        scale_bounds_by_dpr(&mut node, 2.0);
        assert_eq!(node["bounds"]["left"], 20, "parent SHALL be scaled");
        assert_eq!(
            node["children"][0]["bounds"]["left"], 10,
            "child bounds SHALL be scaled recursively"
        );
    }

    // 5. scale_bounds_by_dpr leaves a node without bounds untouched.
    #[test]
    fn scale_ignores_node_without_bounds() {
        let mut node = json!({ "tag": "div" });
        scale_bounds_by_dpr(&mut node, 2.0);
        assert_eq!(node, json!({ "tag": "div" }), "node SHALL be unchanged");
    }

    // 6. scale_bounds_by_dpr skips non-integer bound field values.
    #[test]
    fn scale_skips_non_integer_fields() {
        let mut node = json!({
            "bounds": { "left": "x", "top": 4 }
        });
        scale_bounds_by_dpr(&mut node, 2.0);
        assert_eq!(
            node["bounds"]["left"], "x",
            "non-i64 left SHALL be left as-is"
        );
        assert_eq!(node["bounds"]["top"], 8, "integer top SHALL be scaled");
    }

    // 7. scale_bounds_by_dpr with dpr 1.0 is a no-op on values.
    #[test]
    fn scale_identity_dpr_preserves_values() {
        let mut node = json!({
            "bounds": { "left": 7, "top": 11, "right": 13, "bottom": 17 }
        });
        scale_bounds_by_dpr(&mut node, 1.0);
        let b = &node["bounds"];
        assert_eq!(b["left"], 7, "dpr 1.0 SHALL preserve left");
        assert_eq!(b["bottom"], 17, "dpr 1.0 SHALL preserve bottom");
    }

    // 8. offset_bounds shifts all four edges of `bounds` by dx/dy.
    #[test]
    fn offset_shifts_all_bounds_edges() {
        let mut node = json!({
            "bounds": { "left": 10, "top": 20, "right": 30, "bottom": 40 }
        });
        offset_bounds(&mut node, 5, -3);
        let b = &node["bounds"];
        assert_eq!(b["left"], 15, "left SHALL be offset by dx");
        assert_eq!(b["top"], 17, "top SHALL be offset by dy");
        assert_eq!(b["right"], 35, "right SHALL be offset by dx");
        assert_eq!(b["bottom"], 37, "bottom SHALL be offset by dy");
    }

    // 9. offset_bounds offsets visible_bounds as well as bounds.
    #[test]
    fn offset_shifts_visible_bounds_too() {
        let mut node = json!({
            "bounds": { "left": 0, "top": 0, "right": 0, "bottom": 0 },
            "visible_bounds": { "left": 100, "top": 200, "right": 300, "bottom": 400 }
        });
        offset_bounds(&mut node, 10, 20);
        assert_eq!(node["bounds"]["left"], 10, "bounds SHALL be offset");
        assert_eq!(
            node["visible_bounds"]["top"], 220,
            "visible_bounds SHALL be offset"
        );
    }

    // 10. offset_bounds recurses into children.
    #[test]
    fn offset_recurses_into_children() {
        let mut node = json!({
            "bounds": { "left": 0, "top": 0, "right": 0, "bottom": 0 },
            "children": [
                { "bounds": { "left": 1, "top": 2, "right": 3, "bottom": 4 } }
            ]
        });
        offset_bounds(&mut node, 100, 200);
        assert_eq!(
            node["children"][0]["bounds"]["left"], 101,
            "child left SHALL be offset recursively"
        );
        assert_eq!(
            node["children"][0]["bounds"]["top"], 202,
            "child top SHALL be offset recursively"
        );
    }

    // 11. offset_bounds with zero deltas preserves values.
    #[test]
    fn offset_zero_delta_preserves_values() {
        let mut node = json!({
            "bounds": { "left": 9, "top": 8, "right": 7, "bottom": 6 }
        });
        offset_bounds(&mut node, 0, 0);
        let b = &node["bounds"];
        assert_eq!(b["left"], 9, "zero offset SHALL preserve left");
        assert_eq!(b["bottom"], 6, "zero offset SHALL preserve bottom");
    }

    // 12. offset_bounds leaves a node without bounds untouched.
    #[test]
    fn offset_ignores_node_without_bounds() {
        let mut node = json!({ "tag": "span", "children": [] });
        offset_bounds(&mut node, 5, 5);
        assert_eq!(
            node,
            json!({ "tag": "span", "children": [] }),
            "node without bounds SHALL be unchanged"
        );
    }

    // 13. offset_bounds offsets only the present edges when some are missing.
    #[test]
    fn offset_handles_partial_bounds() {
        let mut node = json!({
            "bounds": { "left": 10, "right": 20 }
        });
        offset_bounds(&mut node, 5, 99);
        assert_eq!(node["bounds"]["left"], 15, "present left SHALL be offset");
        assert_eq!(node["bounds"]["right"], 25, "present right SHALL be offset");
        assert!(
            node["bounds"].get("top").is_none(),
            "absent top SHALL NOT be added"
        );
    }

    // 14. DOM_TRAVERSAL_JS embedded blob is the minified traversal script,
    //     not merely non-whitespace. Assert sentinel substrings that the
    //     traversal logic depends on and that the minifier preserves
    //     (property/global accesses survive name-mangling), proving the
    //     build-script wired up the intended source rather than an empty or
    //     wrong file.
    #[test]
    fn dom_traversal_js_blob_is_the_traversal_script() {
        let js = DOM_TRAVERSAL_JS.trim();
        assert!(
            !js.is_empty(),
            "embedded minified DOM traversal JS SHALL be non-empty"
        );
        assert!(
            js.contains("getBoundingClientRect"),
            "traversal JS SHALL read element bounds via getBoundingClientRect"
        );
        assert!(
            js.contains("IntersectionObserver"),
            "traversal JS SHALL compute visibility via IntersectionObserver"
        );
        assert!(
            js.contains("aria-label"),
            "traversal JS SHALL extract the aria-label attribute"
        );
    }

    // 15. match_webview_socket returns the socket name following the
    //     webview_devtools_remote_<pid> suffix on a matching /proc/net/unix line.
    #[test]
    fn match_socket_finds_name_for_pid() {
        let proc = "0000: 00000002 0 0 0 1 0 12345 @webview_devtools_remote_4321\n";
        let got = match_webview_socket(proc, &["4321"]);
        // The localabstract socket name is matched from the suffix start, so the
        // leading `@` (abstract-namespace marker) is not part of the returned name.
        assert_eq!(
            got.as_deref(),
            Some("webview_devtools_remote_4321"),
            "matching socket name SHALL be returned from the suffix start"
        );
    }

    // 16. match_webview_socket tries PIDs in order and returns the first hit.
    #[test]
    fn match_socket_tries_pids_in_order() {
        let proc = "x y z @webview_devtools_remote_222\nx y z @webview_devtools_remote_111\n";
        // 333 has no socket; 111 does — first listed PID with a match wins.
        let got = match_webview_socket(proc, &["333", "111"]);
        assert_eq!(
            got.as_deref(),
            Some("webview_devtools_remote_111"),
            "first PID with a matching socket SHALL win"
        );
    }

    // 17. match_webview_socket returns None when no PID has a socket.
    #[test]
    fn match_socket_none_when_no_match() {
        let proc = "0000: foo bar @some_other_socket\n";
        assert!(
            match_webview_socket(proc, &["4321"]).is_none(),
            "absent socket SHALL yield None"
        );
    }

    // 18. parse_page_id returns the id of the first target.
    #[test]
    fn parse_page_id_returns_first_target_id() {
        let resp = r#"[{"id":"PAGE_A","type":"page"},{"id":"PAGE_B"}]"#;
        let got = parse_page_id(resp).expect("valid /json SHALL parse");
        assert_eq!(got, "PAGE_A", "the first target's id SHALL be returned");
    }

    // 19. parse_page_id errors on an empty target array.
    #[test]
    fn parse_page_id_errors_on_empty_array() {
        let err = parse_page_id("[]").expect_err("empty array SHALL error");
        assert!(
            err.to_string().contains("No CDP page targets found"),
            "empty array SHALL report no targets, got: {err}"
        );
    }

    // 20. parse_page_id errors when the response is not a JSON array.
    #[test]
    fn parse_page_id_errors_when_not_array() {
        let err = parse_page_id(r#"{"id":"x"}"#).expect_err("object SHALL error");
        assert!(
            err.to_string().contains("not an array"),
            "non-array SHALL report not-an-array, got: {err}"
        );
    }

    // 21. parse_page_id errors when the first target lacks an id.
    #[test]
    fn parse_page_id_errors_without_id() {
        let err = parse_page_id(r#"[{"type":"page"}]"#).expect_err("missing id SHALL error");
        assert!(
            err.to_string().contains("no id"),
            "target without id SHALL report no id, got: {err}"
        );
    }

    // 22. parse_eval_response unquotes a string result value.
    #[test]
    fn parse_eval_unquotes_string_value() {
        let resp = r#"{"id":1,"result":{"result":{"value":"hello"}}}"#;
        let got = parse_eval_response(resp).expect("valid response SHALL parse");
        assert_eq!(
            got,
            Some("hello".to_string()),
            "string value SHALL be returned unquoted"
        );
    }

    // 23. parse_eval_response coerces null and undefined (missing) to empty string.
    #[test]
    fn parse_eval_null_and_missing_become_empty() {
        let null_resp = r#"{"id":1,"result":{"result":{"value":null}}}"#;
        assert_eq!(
            parse_eval_response(null_resp).expect("parse"),
            Some(String::new()),
            "null value SHALL coerce to empty string"
        );
        let missing_resp = r#"{"id":1,"result":{"result":{}}}"#;
        assert_eq!(
            parse_eval_response(missing_resp).expect("parse"),
            Some(String::new()),
            "missing value SHALL coerce to empty string"
        );
    }

    // 24. parse_eval_response stringifies non-string values (numbers, objects).
    #[test]
    fn parse_eval_stringifies_non_string_value() {
        let num_resp = r#"{"id":1,"result":{"result":{"value":42}}}"#;
        assert_eq!(
            parse_eval_response(num_resp).expect("parse"),
            Some("42".to_string()),
            "numeric value SHALL be stringified"
        );
        let obj_resp = r#"{"id":1,"result":{"result":{"value":{"a":1}}}}"#;
        assert_eq!(
            parse_eval_response(obj_resp).expect("parse"),
            Some(r#"{"a":1}"#.to_string()),
            "object value SHALL be JSON-stringified"
        );
    }

    // 25. parse_eval_response returns None for a non-matching id (caller keeps reading).
    #[test]
    fn parse_eval_skips_non_matching_id() {
        let resp = r#"{"id":2,"result":{"result":{"value":"ignored"}}}"#;
        assert_eq!(
            parse_eval_response(resp).expect("parse"),
            None,
            "non-matching id SHALL yield None so the caller keeps reading"
        );
    }

    // 26. parse_eval_response surfaces exceptionDetails as an error.
    #[test]
    fn parse_eval_errors_on_exception_details() {
        let resp = r#"{"id":1,"result":{"exceptionDetails":{"text":"boom"}}}"#;
        let err = parse_eval_response(resp).expect_err("exceptionDetails SHALL error");
        assert!(
            err.to_string().contains("CDP evaluation error"),
            "exceptionDetails SHALL surface as evaluation error, got: {err}"
        );
    }

    // 27. parse_eval_response errors on an unparseable frame.
    #[test]
    fn parse_eval_errors_on_bad_json() {
        let err = parse_eval_response("not json").expect_err("bad json SHALL error");
        assert!(
            err.to_string().contains("failed to parse CDP response"),
            "unparseable frame SHALL report parse failure, got: {err}"
        );
    }
}
