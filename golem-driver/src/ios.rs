use crate::common::{
    build_backspace_body, build_gesture_body, build_long_press_body, build_swipe_body,
    build_tap_body, build_type_body, find_webview_bounds, parse_hierarchy,
    replace_webview_children, CompanionClient,
};
use crate::{PlatformDriver, ScreenshotResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use golem_element::Element;

/// iOS driver that communicates with an XCUITest companion server via HTTP.
///
/// The companion server runs inside the iOS simulator and exposes
/// endpoints for UI automation (tap, type, swipe, screenshot, etc.).
pub struct IosDriver {
    client: CompanionClient,
    device_id: String,
    bundle_id: String,
    /// WebKit Inspector lifecycle for WKWebView DOM access.
    webkit: std::sync::Mutex<WebKitLifecycle>,
}

/// WebKit Inspector connection lifecycle — mirrors `CdpLifecycle` in android.rs.
enum WebKitLifecycle {
    /// Haven't seen a WKWebView yet — no inspector needed.
    Idle,
    /// Background setup task is running.
    SetupInProgress(tokio::sync::oneshot::Receiver<Option<WebKitState>>),
    /// Inspector is connected and ready.
    Ready(WebKitState),
    /// Setup failed — will retry on next WebView sighting.
    Failed,
}

struct WebKitState {
    inspector: crate::webkit::WebKitInspector,
}

/// Convert a `Direction` to swipe coordinate deltas.
///
/// Uses a standard iPhone screen size (390x844) with the center as
/// the origin point and a 200pt gesture distance.
impl IosDriver {
    /// Create a new iOS driver targeting the companion server at the given port.
    pub fn new(device_id: String, bundle_id: String, port: u16) -> Self {
        let client = CompanionClient::new(port);
        Self {
            client,
            device_id,
            bundle_id,
            webkit: std::sync::Mutex::new(WebKitLifecycle::Idle),
        }
    }

    /// Return the base URL for the companion server.
    pub fn base_url(&self) -> &str {
        &self.client.base_url
    }

    /// Check companion server health and return device info.
    pub async fn check_health(&self) -> anyhow::Result<crate::common::CompanionHealth> {
        self.client.check_health().await
    }

    /// Wait for companion to become healthy, polling with timeout.
    pub async fn wait_for_health(&self, timeout: std::time::Duration) -> anyhow::Result<crate::common::CompanionHealth> {
        self.client.wait_for_health(timeout).await
    }

    /// Return the device ID.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Return the bundle ID.
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// Run an `xcrun simctl` subcommand.
    async fn simctl(&self, args: &[&str]) -> Result<String> {
        let output = tokio::process::Command::new("xcrun")
            .arg("simctl")
            .args(args)
            .output()
            .await
            .context("failed to spawn xcrun simctl")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("xcrun simctl {args:?} failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// What to do with WebKit Inspector on this hierarchy call.
enum WebKitAction {
    Skip,
    Enrich(WebKitState),
}

/// Set up WebKit Inspector: discover socket, connect, handshake.
async fn setup_webkit() -> Option<WebKitState> {
    match crate::webkit::WebKitInspector::connect().await {
        Ok(inspector) => Some(WebKitState { inspector }),
        Err(e) => {
            if crate::is_debug() { eprintln!("  [webkit] setup failed: {e}"); }
            None
        }
    }
}

/// Try to enrich a WebView node with WebKit Inspector DOM data.
/// Returns the WebKitState back if successful (for reuse), None if failed.
async fn try_enrich(
    raw: &mut serde_json::Value,
    mut state: WebKitState,
    wv_x: i32,
    wv_y: i32,
) -> Option<WebKitState> {
    match crate::webkit::fetch_webview_dom(&mut state.inspector, wv_x, wv_y).await {
        Some(dom) => {
            replace_webview_children(raw, dom);
            Some(state)
        }
        None => None, // Inspector connection lost
    }
}

#[async_trait]
impl PlatformDriver for IosDriver {
    async fn get_hierarchy(&self) -> Result<(Element, crate::common::HierarchyMeta)> {
        let text = self.client.get_text("/hierarchy").await?;
        let wrapper: serde_json::Value = serde_json::from_str(&text)
            .context("failed to parse hierarchy JSON")?;

        // Extract tree from wrapper (companion sends {"tree": [...], ...})
        let mut raw = wrapper.get("tree").cloned().unwrap_or(wrapper.clone());

        // Check if hierarchy contains a WKWebView
        // CSS getBoundingClientRect() returns coordinates relative to the web
        // viewport, which starts below the safe area. Add safe_area_top so DOM
        // coordinates match the native accessibility tree (screen coordinates).
        let safe_area_top = wrapper
            .get("safe_area_top")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        if let Some((wv_x, wv_y)) = find_webview_bounds(&raw) {
            let wv_y = wv_y + safe_area_top;
            // Check WebKit state (short lock, no async while held)
            let webkit_action = {
                let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                match &mut *wk {
                    WebKitLifecycle::Idle => {
                        // First WebView sighting — kick off background setup
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::spawn(async move {
                            let result = setup_webkit().await;
                            let _ = tx.send(result);
                        });
                        *wk = WebKitLifecycle::SetupInProgress(rx);
                        WebKitAction::Skip
                    }
                    WebKitLifecycle::SetupInProgress(rx) => {
                        match rx.try_recv() {
                            Ok(Some(state)) => {
                                WebKitAction::Enrich(state)
                            }
                            Ok(None) => {
                                *wk = WebKitLifecycle::Failed;
                                WebKitAction::Skip
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                                WebKitAction::Skip // Still setting up
                            }
                            Err(_) => {
                                *wk = WebKitLifecycle::Failed;
                                WebKitAction::Skip
                            }
                        }
                    }
                    WebKitLifecycle::Ready(_) => {
                        // Take ownership of the state for async work
                        let old = std::mem::replace(&mut *wk, WebKitLifecycle::Failed);
                        if let WebKitLifecycle::Ready(state) = old {
                            WebKitAction::Enrich(state)
                        } else {
                            WebKitAction::Skip
                        }
                    }
                    WebKitLifecycle::Failed => {
                        // Retry — the app may have been relaunched
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::spawn(async move {
                            let result = setup_webkit().await;
                            let _ = tx.send(result);
                        });
                        *wk = WebKitLifecycle::SetupInProgress(rx);
                        WebKitAction::Skip
                    }
                }
            }; // mutex dropped here

            // Now do async WebKit work outside the lock
            if let WebKitAction::Enrich(state) = webkit_action {
                if let Some(state) = try_enrich(&mut raw, state, wv_x, wv_y).await {
                    // Put state back
                    let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                    *wk = WebKitLifecycle::Ready(state);
                } else {
                    // Inspector failed — reconnect immediately
                    if let Some(new_state) = setup_webkit().await {
                        if let Some(new_state) =
                            try_enrich(&mut raw, new_state, wv_x, wv_y).await
                        {
                            let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                            *wk = WebKitLifecycle::Ready(new_state);
                        } else {
                            let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                            *wk = WebKitLifecycle::Failed;
                        }
                    } else {
                        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                        *wk = WebKitLifecycle::Failed;
                    }
                }
            }
        }

        // Reconstruct wrapper with enriched tree for parse_hierarchy
        let original: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let mut response = serde_json::json!({ "tree": raw });
        if let Some(obj) = original.as_object() {
            for key in [
                "keyboard_height",
                "safe_area_top",
                "safe_area_bottom",
                "device_model",
            ] {
                if let Some(val) = obj.get(key) {
                    response[key] = val.clone();
                }
            }
        }
        let enriched_str =
            serde_json::to_string(&response).context("failed to serialize hierarchy")?;
        parse_hierarchy(&enriched_str)
    }

    async fn tap(&self, x: i32, y: i32) -> Result<()> {
        let body = build_tap_body(x, y)?;
        self.client.post_json("/tap", &body).await?;
        Ok(())
    }

    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> Result<()> {
        let body = build_long_press_body(x, y, duration_ms)?;
        self.client.post_json("/longpress", &body).await?;
        Ok(())
    }

    async fn type_text(&self, text: &str) -> Result<()> {
        let body = build_type_body(text)?;
        self.client.post_json("/type", &body).await?;
        Ok(())
    }

    async fn backspace(&self, count: u32) -> Result<()> {
        let body = build_backspace_body(count)?;
        self.client.post_json("/backspace", &body).await?;
        Ok(())
    }

    async fn swipe_coords(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<()> {
        let body = build_swipe_body(from_x, from_y, to_x, to_y, 300)?;
        self.client.post_json("/swipe", &body).await?;
        Ok(())
    }

    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> Result<()> {
        let body = serde_json::json!({ "x": x, "y": y, "scale": scale, "velocity": velocity }).to_string();
        self.client.post_json("/pinch", &body).await?;
        Ok(())
    }

    async fn gesture(&self, fingers: Vec<crate::GestureFinger>) -> Result<()> {
        let body = build_gesture_body(&fingers)?;
        self.client.post_json("/gesture", &body).await?;
        Ok(())
    }

    async fn screenshot(&self) -> Result<ScreenshotResult> {
        let data = self.client.get_bytes("/screenshot").await?;
        Ok(ScreenshotResult {
            path: String::new(),
            data,
        })
    }

    async fn hide_keyboard(&self) -> Result<()> {
        self.client.post_json("/hide-keyboard", "{}").await?;
        Ok(())
    }

    async fn launch_app(&self, bundle_id: &str) -> Result<()> {
        let body = serde_json::json!({ "bundle_id": bundle_id }).to_string();
        self.client.post_json("/launch", &body).await?;
        // Reset WebKit Inspector — the target app may have changed, or
        // the inspector session may be stale after an app switch.
        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
        *wk = WebKitLifecycle::Idle;
        Ok(())
    }

    async fn stop_app(&self, bundle_id: &str) -> Result<()> {
        let body = serde_json::json!({ "bundle_id": bundle_id }).to_string();
        self.client.post_json("/stop", &body).await?;
        // App process is gone — the old inspector connection is dead.
        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
        *wk = WebKitLifecycle::Idle;
        Ok(())
    }

    async fn clear_app_data(&self, bundle_id: &str) -> Result<()> {
        // Uninstall and reinstall is the standard way to clear data on iOS simulator
        self.simctl(&["uninstall", &self.device_id, bundle_id])
            .await?;
        Ok(())
    }

    async fn press_button(&self, button: &str) -> Result<()> {
        match button {
            "home" => {
                self.simctl(&["ui", &self.device_id, "home"]).await?;
            }
            other => {
                anyhow::bail!("unsupported button on iOS: {other}");
            }
        }
        Ok(())
    }

    async fn set_orientation(&self, orientation: &str) -> Result<()> {
        // Orientation is not directly supported via simctl; would need companion endpoint
        anyhow::bail!("set_orientation is not yet supported on iOS (requested: {orientation})")
    }

    async fn set_dark_mode(&self, enabled: bool) -> Result<()> {
        let style = if enabled { "dark" } else { "light" };
        self.simctl(&["ui", &self.device_id, "appearance", style])
            .await?;
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> Result<()> {
        self.simctl(&[
            "location",
            &self.device_id,
            "set",
            &format!("{lat},{lon}"),
        ])
        .await?;
        Ok(())
    }

    async fn open_url(&self, url: &str) -> Result<()> {
        self.simctl(&["openurl", &self.device_id, url]).await?;
        Ok(())
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        payload: Option<&str>,
    ) -> Result<()> {
        // Build an APNS payload JSON and push via simctl
        let apns_payload = if let Some(custom) = payload {
            format!(
                r#"{{"aps":{{"alert":{{"title":"{title}","body":"{body}"}}}},"custom":{custom}}}"#
            )
        } else {
            format!(r#"{{"aps":{{"alert":{{"title":"{title}","body":"{body}"}}}}}}"#)
        };

        // Write payload to a temp file, then push
        let tmp = std::env::temp_dir().join(format!("golem_push_{}.json", std::process::id()));
        tokio::fs::write(&tmp, &apns_payload)
            .await
            .context("writing push payload to temp file")?;

        let tmp_str = tmp
            .to_str()
            .context("temp path is not valid UTF-8")?;

        let result = self
            .simctl(&["push", &self.device_id, &self.bundle_id, tmp_str])
            .await;

        // Clean up temp file regardless of result
        let _ = tokio::fs::remove_file(&tmp).await;

        result?;
        Ok(())
    }

    async fn add_media(&self, path: &str) -> Result<()> {
        self.simctl(&["addmedia", &self.device_id, path]).await?;
        Ok(())
    }

    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> Result<()> {
        self.simctl(&[
            "privacy",
            &self.device_id,
            "grant",
            permission,
            bundle_id,
        ])
        .await?;
        Ok(())
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> Result<()> {
        self.simctl(&[
            "privacy",
            &self.device_id,
            "revoke",
            permission,
            bundle_id,
        ])
        .await?;
        Ok(())
    }

    async fn start_recording(&self, _name: &str) -> Result<()> {
        // Recording via simctl requires a long-running process; not yet implemented
        anyhow::bail!("start_recording is not yet supported on iOS")
    }

    async fn stop_recording(&self) -> Result<String> {
        anyhow::bail!("stop_recording is not yet supported on iOS")
    }



    async fn remove_port_forwards(&self) -> Result<()> {
        Ok(()) // Not applicable to iOS
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::Bounds;

    // -----------------------------------------------------------------------
    // 1. Parse hierarchy JSON into Element tree
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_basic() {
        let json = r#"{
            "element_type": "View",
            "text": null,
            "id": "root",
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 390, "height": 844 },
            "children": [
                {
                    "element_type": "Button",
                    "text": "Login",
                    "id": "login_btn",
                    "placeholder": null,
                    "enabled": true,
                    "checked": false,
                    "clickable": true,
                    "focused": false,
                    "bounds": { "x": 100, "y": 400, "width": 190, "height": 44 },
                    "children": []
                }
            ]
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.0.element_type, "View");
        assert_eq!(element.0.accessibility_label.as_deref(), Some("root"));
        assert_eq!(element.0.children.len(), 1);

        let btn = &element.0.children[0];
        assert_eq!(btn.element_type, "Button");
        assert_eq!(btn.text.as_deref(), Some("Login"));
        assert_eq!(btn.accessibility_label.as_deref(), Some("login_btn"));
        assert!(btn.clickable);
        assert_eq!(btn.bounds, Bounds::new(100, 400, 190, 44));
    }

    // -----------------------------------------------------------------------
    // 2. Parse empty hierarchy (leaf element with no children)
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_empty_children() {
        let json = r#"{
            "element_type": "View",
            "text": null,
            "id": null,
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 375, "height": 812 }
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.0.element_type, "View");
        assert!(element.0.children.is_empty());
    }

    // -----------------------------------------------------------------------
    // 3. Parse hierarchy with nested children
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_deeply_nested() {
        let json = r#"{
            "element_type": "Window",
            "text": null,
            "id": null,
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 390, "height": 844 },
            "children": [{
                "element_type": "View",
                "text": null,
                "id": "container",
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": false,
                "focused": false,
                "bounds": { "x": 0, "y": 44, "width": 390, "height": 800 },
                "children": [{
                    "element_type": "Label",
                    "text": "Hello",
                    "id": null,
                    "placeholder": null,
                    "enabled": true,
                    "checked": false,
                    "clickable": false,
                    "focused": false,
                    "bounds": { "x": 20, "y": 100, "width": 350, "height": 24 },
                    "children": []
                }]
            }]
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.0.element_type, "Window");
        assert_eq!(element.0.children.len(), 1);
        assert_eq!(element.0.children[0].element_type, "View");
        assert_eq!(element.0.children[0].children.len(), 1);
        assert_eq!(element.0.children[0].children[0].element_type, "Label");
        assert_eq!(
            element.0.children[0].children[0].text.as_deref(),
            Some("Hello")
        );
    }

    // -----------------------------------------------------------------------
    // 4. Parse screenshot response (bytes)
    // -----------------------------------------------------------------------
    #[test]
    fn screenshot_result_from_bytes() {
        let png_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let result = ScreenshotResult {
            path: String::new(),
            data: png_bytes.clone(),
        };
        assert_eq!(result.data, png_bytes);
        assert!(result.path.is_empty());
    }

    // -----------------------------------------------------------------------
    // 5. Tap request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn tap_request_serialization() {
        let body = build_tap_body(150, 300).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["x"], 150);
        assert_eq!(parsed["y"], 300);
    }

    // -----------------------------------------------------------------------
    // 6. Type text request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn type_text_request_serialization() {
        let body = build_type_body("hello world").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["text"], "hello world");
    }

    #[test]
    fn type_text_request_with_special_chars() {
        let body = build_type_body("line1\nline2\ttab").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["text"], "line1\nline2\ttab");
    }

    // -----------------------------------------------------------------------
    // 7. Swipe body serialization
    // -----------------------------------------------------------------------
    #[test]
    fn swipe_body_serialization() {
        let body = build_swipe_body(10, 20, 30, 40, 500).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["from_x"], 10);
        assert_eq!(parsed["from_y"], 20);
        assert_eq!(parsed["to_x"], 30);
        assert_eq!(parsed["to_y"], 40);
        assert_eq!(parsed["duration_ms"], 500);
    }

    // -----------------------------------------------------------------------
    // 8. Long press request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn long_press_request_serialization() {
        let body = build_long_press_body(200, 400, 1500).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["x"], 200);
        assert_eq!(parsed["y"], 400);
        assert_eq!(parsed["duration_ms"], 1500);
    }

    // -----------------------------------------------------------------------
    // 9. IosDriver new() sets correct base URL
    // -----------------------------------------------------------------------
    #[test]
    fn ios_driver_new_sets_base_url() {
        let driver = IosDriver::new(
            "ABCD-1234".to_string(),
            "com.example.app".to_string(),
            8222,
        );
        assert_eq!(driver.base_url(), "http://localhost:8222");
        assert_eq!(driver.device_id(), "ABCD-1234");
        assert_eq!(driver.bundle_id(), "com.example.app");
    }

    #[test]
    fn ios_driver_new_custom_port() {
        let driver = IosDriver::new(
            "device-99".to_string(),
            "com.test.bundle".to_string(),
            9999,
        );
        assert_eq!(driver.base_url(), "http://localhost:9999");
    }

    // -----------------------------------------------------------------------
    // Additional: backspace request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn backspace_request_serialization() {
        let body = build_backspace_body(5).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["count"], 5);
    }
}
