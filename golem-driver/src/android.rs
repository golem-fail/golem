use crate::common::{
    build_alert_body, build_backspace_body, build_long_press_body, build_swipe_body,
    build_tap_body, build_type_body, find_alert, parse_hierarchy, CompanionClient, SwipeRequest,
};
use crate::{Direction, PlatformDriver, ScreenshotResult};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use golem_element::Element;

/// Android driver that communicates with an Instrumentation companion server via HTTP.
///
/// The companion server runs on the Android device/emulator (port 8223, forwarded
/// via `adb forward`) and exposes endpoints for UI automation (tap, type, swipe,
/// screenshot, etc.) — the same surface as the iOS companion.
pub struct AndroidDriver {
    client: CompanionClient,
    device_serial: String,
    package_name: String,
    /// CDP lifecycle: None → SetupInProgress → Ready | Failed
    cdp: std::sync::Mutex<CdpLifecycle>,
}

enum CdpLifecycle {
    /// Haven't seen a WebView yet — no CDP needed.
    Idle,
    /// Background setup task is running.
    SetupInProgress(tokio::sync::oneshot::Receiver<Option<CdpState>>),
    /// CDP is ready — use for enrichment.
    Ready(CdpState),
    /// CDP setup failed — don't retry.
    Failed,
}

struct CdpState {
    port: u16,
    page_id: String,
}

/// Convert a `Direction` to swipe coordinate deltas.
///
/// Uses a standard Android phone screen size (1080x2400 in dp-equivalent
/// ~360x800) with the center as the origin point and a 200pt gesture distance.
fn direction_to_swipe_coords(direction: Direction) -> SwipeRequest {
    let center_x: i32 = 180;
    let center_y: i32 = 400;
    let distance: i32 = 200;
    let duration_ms: u64 = 300;

    match direction {
        Direction::Up => SwipeRequest {
            from_x: center_x,
            from_y: center_y + distance / 2,
            to_x: center_x,
            to_y: center_y - distance / 2,
            duration_ms,
        },
        Direction::Down => SwipeRequest {
            from_x: center_x,
            from_y: center_y - distance / 2,
            to_x: center_x,
            to_y: center_y + distance / 2,
            duration_ms,
        },
        Direction::Left => SwipeRequest {
            from_x: center_x + distance / 2,
            from_y: center_y,
            to_x: center_x - distance / 2,
            to_y: center_y,
            duration_ms,
        },
        Direction::Right => SwipeRequest {
            from_x: center_x - distance / 2,
            from_y: center_y,
            to_x: center_x + distance / 2,
            to_y: center_y,
            duration_ms,
        },
    }
}

/// Build the adb command arguments for launching an app via monkey.
#[cfg(test)]
fn build_launch_app_args(serial: &str, package: &str) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "monkey".to_string(),
        "-p".to_string(),
        package.to_string(),
        "-c".to_string(),
        "android.intent.category.LAUNCHER".to_string(),
        "1".to_string(),
    ]
}

/// Build the adb command arguments for setting location.
///
/// Note: the `emu geo fix` command takes longitude before latitude.
#[cfg(test)]
fn build_set_location_args(serial: &str, lat: f64, lon: f64) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "emu".to_string(),
        "geo".to_string(),
        "fix".to_string(),
        lon.to_string(),
        lat.to_string(),
    ]
}

impl AndroidDriver {
    /// Create a new Android driver targeting the companion server at the given port.
    pub fn new(device_serial: String, package_name: String, port: u16) -> Self {
        Self {
            client: CompanionClient::new(port),
            device_serial,
            package_name,
            cdp: std::sync::Mutex::new(CdpLifecycle::Idle),
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

    /// Return the device serial.
    pub fn device_serial(&self) -> &str {
        &self.device_serial
    }

    /// Return the package name.
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    /// Run an `adb` subcommand targeting this device.
    async fn adb(&self, args: &[&str]) -> Result<String> {
        let output = tokio::process::Command::new("adb")
            .arg("-s")
            .arg(&self.device_serial)
            .args(args)
            .output()
            .await
            .context("failed to spawn adb")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("adb -s {} {args:?} failed: {stderr}", self.device_serial);
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// What to do with CDP on this hierarchy call.
enum CdpAction {
    Skip,
    Enrich(u16, String), // port, page_id
}

/// Set up CDP: discover socket, ADB forward, get page ID.
async fn setup_cdp(device_serial: &str) -> Option<CdpState> {
    let socket_name = crate::cdp::find_webview_socket(device_serial).await?;
    let port = crate::cdp::setup_forward(device_serial, &socket_name).await.ok()?;
    let page_id = crate::cdp::get_page_id(port).await.ok()?;
    Some(CdpState { port, page_id })
}

/// Try to enrich a WebView node with CDP DOM data.
/// Returns false if CDP failed (caller should reset state for recovery).
async fn try_enrich(raw: &mut serde_json::Value, port: u16, page_id: &str, wv_left: i32, wv_top: i32) -> bool {
    let dom_json = match crate::cdp::evaluate_dom_js_cached(port, page_id).await {
        Ok(json) => json,
        Err(_) => return false, // Dead socket — signal recovery
    };

    let wrapper = match serde_json::from_str::<serde_json::Value>(&dom_json) {
        Ok(w) => w,
        Err(_) => return false,
    };

    if let Some(meta) = wrapper.get("meta") {
        eprintln!(
            "  [cdp] DOM traversal: {}ms, {} nodes, dpr={}, url={}",
            meta.get("elapsed_ms").and_then(|v| v.as_i64()).unwrap_or(-1),
            meta.get("node_count").and_then(|v| v.as_i64()).unwrap_or(-1),
            meta.get("dpr").and_then(|v| v.as_f64()).unwrap_or(-1.0),
            meta.get("url").and_then(|v| v.as_str()).unwrap_or("?"),
        );
    }

    if let Some(mut tree) = wrapper.get("tree").cloned() {
        crate::cdp::offset_bounds(&mut tree, wv_left, wv_top);
        replace_webview_children(raw, tree);
    }
    true
}

/// Find the first android.webkit.WebView in the hierarchy and return its bounds (left, top).
fn find_webview_bounds(val: &serde_json::Value) -> Option<(i32, i32)> {
    if let Some(cls) = val.get("class").and_then(|v| v.as_str()) {
        if cls == "android.webkit.WebView" {
            let bounds = val.get("bounds")?;
            let left = bounds.get("left").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let top = bounds.get("top").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            return Some((left, top));
        }
    }
    if let Some(children) = val.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(bounds) = find_webview_bounds(child) {
                return Some(bounds);
            }
        }
    }
    None
}

/// Replace the first android.webkit.WebView's children with CDP DOM data.
fn replace_webview_children(val: &mut serde_json::Value, cdp_dom: serde_json::Value) -> bool {
    if let Some(cls) = val.get("class").and_then(|v| v.as_str()) {
        if cls == "android.webkit.WebView" {
            if let Some(children) = val.get_mut("children").and_then(|c| c.as_array_mut()) {
                children.clear();
                children.push(cdp_dom);
            }
            return true;
        }
    }
    if let Some(children) = val.get_mut("children").and_then(|c| c.as_array_mut()) {
        for child in children {
            if replace_webview_children(child, cdp_dom.clone()) {
                return true;
            }
        }
    }
    false
}

#[async_trait]
impl PlatformDriver for AndroidDriver {
    async fn get_hierarchy(&self) -> Result<Element> {
        let text = self.client.get_text("/hierarchy").await?;
        let mut raw: serde_json::Value = serde_json::from_str(&text)
            .context("failed to parse hierarchy JSON")?;

        // Check if hierarchy contains a WebView
        if let Some((wv_left, wv_top)) = find_webview_bounds(&raw) {
            // Check CDP state (short lock, no async while held)
            let cdp_action = {
                let mut cdp = self.cdp.lock().expect("cdp mutex poisoned");
                match &mut *cdp {
                    CdpLifecycle::Idle => {
                        // First WebView sighting — kick off background setup
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let serial = self.device_serial.clone();
                        tokio::spawn(async move {
                            let result = setup_cdp(&serial).await;
                            let _ = tx.send(result);
                        });
                        *cdp = CdpLifecycle::SetupInProgress(rx);
                        CdpAction::Skip // Return accessibility tree this time
                    }
                    CdpLifecycle::SetupInProgress(rx) => {
                        match rx.try_recv() {
                            Ok(Some(state)) => {
                                let port = state.port;
                                let page_id = state.page_id.clone();
                                *cdp = CdpLifecycle::Ready(state);
                                CdpAction::Enrich(port, page_id)
                            }
                            Ok(None) => {
                                *cdp = CdpLifecycle::Failed;
                                CdpAction::Skip
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                                CdpAction::Skip // Still setting up
                            }
                            Err(_) => {
                                *cdp = CdpLifecycle::Failed;
                                CdpAction::Skip
                            }
                        }
                    }
                    CdpLifecycle::Ready(state) => {
                        CdpAction::Enrich(state.port, state.page_id.clone())
                    }
                    CdpLifecycle::Failed => CdpAction::Skip,
                }
            }; // mutex dropped here

            // Now do async CDP work outside the lock
            if let CdpAction::Enrich(port, page_id) = cdp_action {
                if !try_enrich(&mut raw, port, &page_id, wv_left, wv_top).await {
                    // CDP failed (dead socket, app restart) — reset to Idle for auto-recovery
                    let mut cdp = self.cdp.lock().expect("cdp mutex poisoned");
                    *cdp = CdpLifecycle::Idle;
                }
            }
        }

        let enriched_str = serde_json::to_string(&raw)
            .context("failed to serialize hierarchy")?;
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

    async fn swipe(&self, direction: Direction) -> Result<()> {
        let req = direction_to_swipe_coords(direction);
        let body = serde_json::to_string(&req).context("failed to serialize swipe request")?;
        self.client.post_json("/swipe", &body).await?;
        Ok(())
    }

    async fn swipe_coords(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<()> {
        let body = build_swipe_body(from_x, from_y, to_x, to_y, 300)?;
        self.client.post_json("/swipe", &body).await?;
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
        // Use adb shell directly — uiAutomation.executeShellCommand doesn't
        // reliably launch apps on all Android versions.
        self.adb(&[
            "shell", "am", "start",
            "-a", "android.intent.action.MAIN",
            "-c", "android.intent.category.LAUNCHER",
            "-n", &format!("{bundle_id}/.MainActivity"),
        ]).await?;
        Ok(())
    }

    async fn stop_app(&self, bundle_id: &str) -> Result<()> {
        self.adb(&["shell", "am", "force-stop", bundle_id]).await?;
        Ok(())
    }

    async fn clear_app_data(&self, bundle_id: &str) -> Result<()> {
        self.adb(&["shell", "pm", "clear", bundle_id]).await?;
        Ok(())
    }

    async fn press_button(&self, button: &str) -> Result<()> {
        let keyevent = match button {
            "home" => "HOME",
            "back" => "BACK",
            "volume_up" => "VOLUME_UP",
            "volume_down" => "VOLUME_DOWN",
            other => {
                anyhow::bail!("unsupported button on Android: {other}");
            }
        };
        self.adb(&["shell", "input", "keyevent", keyevent]).await?;
        Ok(())
    }

    async fn set_orientation(&self, orientation: &str) -> Result<()> {
        let value = match orientation {
            "portrait" => "0",
            "landscape" => "1",
            "reverse_portrait" => "2",
            "reverse_landscape" => "3",
            other => {
                anyhow::bail!("unsupported orientation on Android: {other}");
            }
        };
        self.adb(&[
            "shell",
            "settings",
            "put",
            "system",
            "user_rotation",
            value,
        ])
        .await?;
        Ok(())
    }

    async fn set_dark_mode(&self, enabled: bool) -> Result<()> {
        let mode = if enabled { "yes" } else { "no" };
        self.adb(&["shell", "cmd", "uimode", "night", mode]).await?;
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> Result<()> {
        // Note: `emu geo fix` takes longitude before latitude
        self.adb(&["emu", "geo", "fix", &lon.to_string(), &lat.to_string()])
            .await?;
        Ok(())
    }

    async fn open_url(&self, url: &str) -> Result<()> {
        self.adb(&[
            "shell",
            "am",
            "start",
            "-a",
            "android.intent.action.VIEW",
            "-d",
            url,
        ])
        .await?;
        Ok(())
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        _payload: Option<&str>,
    ) -> Result<()> {
        // Android push notifications via adb require a broadcast receiver in the app.
        // Use a simple am broadcast approach.
        self.adb(&[
            "shell",
            "am",
            "broadcast",
            "-a",
            "fail.golem.PUSH_NOTIFICATION",
            "--es",
            "title",
            title,
            "--es",
            "body",
            body,
            "-n",
            &format!("{}/fail.golem.PushReceiver", self.package_name),
        ])
        .await?;
        Ok(())
    }

    async fn add_media(&self, path: &str) -> Result<()> {
        self.adb(&["push", path, "/sdcard/DCIM/"]).await?;
        Ok(())
    }

    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> Result<()> {
        self.adb(&["shell", "pm", "grant", bundle_id, permission])
            .await?;
        Ok(())
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> Result<()> {
        self.adb(&["shell", "pm", "revoke", bundle_id, permission])
            .await?;
        Ok(())
    }

    async fn start_recording(&self, name: &str) -> Result<()> {
        let path = format!("/sdcard/{name}.mp4");
        // screenrecord runs in background; we detach it using nohup
        self.adb(&["shell", "screenrecord", &path]).await?;
        Ok(())
    }

    async fn stop_recording(&self) -> Result<String> {
        // Kill the screenrecord process
        self.adb(&["shell", "pkill", "-INT", "screenrecord"])
            .await?;
        Ok("recording.mp4".to_string())
    }

    async fn get_alert(&self) -> Result<Option<Element>> {
        let text = self.client.get_text("/hierarchy").await?;
        let root = parse_hierarchy(&text)?;
        Ok(find_alert(&root))
    }

    async fn dismiss_alert(&self, button: Option<&str>) -> Result<()> {
        let action = match button {
            Some("OK") | Some("Accept") | Some("Yes") | Some("Allow") => "accept",
            _ => "dismiss",
        };
        let body = build_alert_body(action)?;
        self.client.post_json("/alert", &body).await?;
        Ok(())
    }

    async fn remove_port_forwards(&self) -> Result<()> {
        let output = tokio::process::Command::new("adb")
            .args(["-s", &self.device_serial, "forward", "--remove-all"])
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("adb forward --remove-all failed: {stderr}");
        }
        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::parse_alert_response;
    use golem_element::Bounds;

    // -----------------------------------------------------------------------
    // 1. Parse hierarchy JSON into Element tree
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_basic() {
        let json = r#"{
            "element_type": "FrameLayout",
            "text": null,
            "id": "root",
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 1080, "height": 2400 },
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
                    "bounds": { "x": 100, "y": 400, "width": 880, "height": 120 },
                    "children": []
                }
            ]
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.element_type, "FrameLayout");
        assert_eq!(element.accessibility_id.as_deref(), Some("root"));
        assert_eq!(element.children.len(), 1);

        let btn = &element.children[0];
        assert_eq!(btn.element_type, "Button");
        assert_eq!(btn.text.as_deref(), Some("Login"));
        assert_eq!(btn.accessibility_id.as_deref(), Some("login_btn"));
        assert!(btn.clickable);
        assert_eq!(btn.bounds, Bounds::new(100, 400, 880, 120));
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
            "bounds": { "x": 0, "y": 0, "width": 1080, "height": 2400 }
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.element_type, "View");
        assert!(element.children.is_empty());
    }

    // -----------------------------------------------------------------------
    // 3. Parse hierarchy with nested children
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_deeply_nested() {
        let json = r#"{
            "element_type": "DecorView",
            "text": null,
            "id": null,
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 1080, "height": 2400 },
            "children": [{
                "element_type": "LinearLayout",
                "text": null,
                "id": "container",
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": false,
                "focused": false,
                "bounds": { "x": 0, "y": 63, "width": 1080, "height": 2337 },
                "children": [{
                    "element_type": "TextView",
                    "text": "Hello",
                    "id": null,
                    "placeholder": null,
                    "enabled": true,
                    "checked": false,
                    "clickable": false,
                    "focused": false,
                    "bounds": { "x": 20, "y": 100, "width": 1040, "height": 48 },
                    "children": []
                }]
            }]
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.element_type, "DecorView");
        assert_eq!(element.children.len(), 1);
        assert_eq!(element.children[0].element_type, "LinearLayout");
        assert_eq!(element.children[0].children.len(), 1);
        assert_eq!(element.children[0].children[0].element_type, "TextView");
        assert_eq!(
            element.children[0].children[0].text.as_deref(),
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
    // 5. Tap request body serialization
    // -----------------------------------------------------------------------
    #[test]
    fn tap_request_serialization() {
        let body = build_tap_body(540, 1200).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["x"], 540);
        assert_eq!(parsed["y"], 1200);
    }

    // -----------------------------------------------------------------------
    // 6. Type text request body serialization
    // -----------------------------------------------------------------------
    #[test]
    fn type_text_request_serialization() {
        let body = build_type_body("hello android").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["text"], "hello android");
    }

    #[test]
    fn type_text_request_with_special_chars() {
        let body = build_type_body("line1\nline2\ttab").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["text"], "line1\nline2\ttab");
    }

    // -----------------------------------------------------------------------
    // 7. Swipe request with Direction conversion
    // -----------------------------------------------------------------------
    #[test]
    fn swipe_direction_up() {
        let req = direction_to_swipe_coords(Direction::Up);
        assert!(req.from_y > req.to_y, "swiping up means from_y > to_y");
        assert_eq!(req.from_x, req.to_x, "vertical swipe keeps x constant");
    }

    #[test]
    fn swipe_direction_down() {
        let req = direction_to_swipe_coords(Direction::Down);
        assert!(req.from_y < req.to_y, "swiping down means from_y < to_y");
        assert_eq!(req.from_x, req.to_x, "vertical swipe keeps x constant");
    }

    #[test]
    fn swipe_direction_left() {
        let req = direction_to_swipe_coords(Direction::Left);
        assert!(
            req.from_x > req.to_x,
            "swiping left means from_x > to_x"
        );
        assert_eq!(req.from_y, req.to_y, "horizontal swipe keeps y constant");
    }

    #[test]
    fn swipe_direction_right() {
        let req = direction_to_swipe_coords(Direction::Right);
        assert!(
            req.from_x < req.to_x,
            "swiping right means from_x < to_x"
        );
        assert_eq!(req.from_y, req.to_y, "horizontal swipe keeps y constant");
    }

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
    // 8. Long press request body serialization
    // -----------------------------------------------------------------------
    #[test]
    fn long_press_request_serialization() {
        let body = build_long_press_body(540, 1200, 2000).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["x"], 540);
        assert_eq!(parsed["y"], 1200);
        assert_eq!(parsed["duration_ms"], 2000);
    }

    // -----------------------------------------------------------------------
    // 9. AndroidDriver new() sets correct base URL
    // -----------------------------------------------------------------------
    #[test]
    fn android_driver_new_sets_base_url() {
        let driver = AndroidDriver::new(
            "emulator-5554".to_string(),
            "com.example.myapp".to_string(),
            8223,
        );
        assert_eq!(driver.base_url(), "http://localhost:8223");
        assert_eq!(driver.device_serial(), "emulator-5554");
        assert_eq!(driver.package_name(), "com.example.myapp");
    }

    #[test]
    fn android_driver_new_custom_port() {
        let driver = AndroidDriver::new(
            "device-abc123".to_string(),
            "com.test.app".to_string(),
            9999,
        );
        assert_eq!(driver.base_url(), "http://localhost:9999");
    }

    // -----------------------------------------------------------------------
    // 10. Parse alert response
    // -----------------------------------------------------------------------
    #[test]
    fn parse_alert_response_with_alert() {
        let json = r#"{
            "alert": {
                "element_type": "Alert",
                "text": "Allow access?",
                "id": null,
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": true,
                "focused": false,
                "bounds": { "x": 80, "y": 600, "width": 920, "height": 400 },
                "children": []
            }
        }"#;

        let alert = parse_alert_response(json).expect("should parse");
        assert!(alert.is_some());
        let el = alert.expect("alert present");
        assert_eq!(el.element_type, "Alert");
        assert_eq!(el.text.as_deref(), Some("Allow access?"));
    }

    #[test]
    fn parse_alert_response_no_alert() {
        let json = r#"{ "alert": null }"#;
        let alert = parse_alert_response(json).expect("should parse");
        assert!(alert.is_none());
    }

    #[test]
    fn alert_body_accept() {
        let body = build_alert_body("accept").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["action"], "accept");
    }

    #[test]
    fn alert_body_dismiss() {
        let body = build_alert_body("dismiss").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["action"], "dismiss");
    }

    // -----------------------------------------------------------------------
    // 11. ADB command construction for launch_app
    // -----------------------------------------------------------------------
    #[test]
    fn adb_launch_app_args() {
        let args = build_launch_app_args("emulator-5554", "com.example.app");
        assert_eq!(
            args,
            vec![
                "-s",
                "emulator-5554",
                "shell",
                "monkey",
                "-p",
                "com.example.app",
                "-c",
                "android.intent.category.LAUNCHER",
                "1",
            ]
        );
    }

    #[test]
    fn adb_launch_app_args_with_different_serial() {
        let args = build_launch_app_args("192.168.1.100:5555", "org.test.sample");
        assert_eq!(args[1], "192.168.1.100:5555");
        assert_eq!(args[5], "org.test.sample");
    }

    // -----------------------------------------------------------------------
    // 12. ADB command construction for set_location (lon before lat)
    // -----------------------------------------------------------------------
    #[test]
    fn adb_set_location_args_lon_before_lat() {
        let args = build_set_location_args("emulator-5554", 37.7749, -122.4194);
        // emu geo fix <lon> <lat> — longitude comes first!
        assert_eq!(args[0], "-s");
        assert_eq!(args[1], "emulator-5554");
        assert_eq!(args[2], "emu");
        assert_eq!(args[3], "geo");
        assert_eq!(args[4], "fix");
        assert_eq!(args[5], "-122.4194"); // longitude first
        assert_eq!(args[6], "37.7749"); // latitude second
    }

    #[test]
    fn adb_set_location_args_positive_coords() {
        let args = build_set_location_args("device-1", 51.5074, 0.1278);
        assert_eq!(args[5], "0.1278"); // longitude first
        assert_eq!(args[6], "51.5074"); // latitude second
    }

    // -----------------------------------------------------------------------
    // Additional: backspace request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn backspace_request_serialization() {
        let body = build_backspace_body(7).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["count"], 7);
    }
}
