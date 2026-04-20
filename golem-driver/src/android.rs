use crate::common::{
    build_backspace_body, build_gesture_body, build_long_press_body, build_swipe_body,
    build_tap_body, build_type_body, parse_hierarchy, CompanionClient,
};
use crate::{PlatformDriver, ScreenshotResult};
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
/// Cleans up any previous forward before creating a new one.
async fn setup_cdp(device_serial: &str) -> Option<CdpState> {
    // Clean up stale CDP forwards from previous sessions
    crate::cdp::cleanup_stale_forwards(device_serial).await;

    let socket_name = crate::cdp::find_webview_socket(device_serial).await?;
    let port = crate::cdp::setup_forward(device_serial, &socket_name).await.ok()?;
    let page_id = match crate::cdp::get_page_id(port).await {
        Ok(id) => id,
        Err(e) => {
            eprintln!("  [cdp] setup failed at get_page_id: {e}");
            let _ = crate::cdp::remove_forward(device_serial, port).await;
            return None;
        }
    };
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
        let url = meta.get("url").and_then(|v| v.as_str()).unwrap_or("");

        // Skip enrichment if the page hasn't loaded yet — don't replace
        // valid accessibility tree data with an empty/blank page.
        if url == "about:blank" || url.is_empty() {
            return true; // CDP is working, just not ready — don't reset state
        }
    }

    if let Some(mut tree) = wrapper.get("tree").cloned() {
        // JS reports CSS pixels; Android accessibility tree uses device pixels.
        // Scale by dpr to match.
        let dpr = wrapper
            .get("meta")
            .and_then(|m| m.get("dpr"))
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        if dpr > 1.0 {
            crate::cdp::scale_bounds_by_dpr(&mut tree, dpr);
        }
        crate::cdp::offset_bounds(&mut tree, wv_left, wv_top);
        replace_webview_children(raw, tree);
    }
    true
}

use crate::common::{find_webview_bounds, replace_webview_children};

#[async_trait]
impl PlatformDriver for AndroidDriver {
    async fn get_hierarchy(&self) -> Result<(Element, crate::common::HierarchyMeta)> {
        let text = self.client.get_text("/hierarchy").await?;
        let wrapper: serde_json::Value = serde_json::from_str(&text)
            .context("failed to parse hierarchy JSON")?;

        // Extract tree from wrapper (companion sends {"tree": {...}, "keyboard_height": N})
        let mut raw = wrapper.get("tree").cloned().unwrap_or(wrapper);

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
                    CdpLifecycle::Failed => {
                        // Retry — the app may have been relaunched with a new WebView
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let serial = self.device_serial.clone();
                        tokio::spawn(async move {
                            let result = setup_cdp(&serial).await;
                            let _ = tx.send(result);
                        });
                        *cdp = CdpLifecycle::SetupInProgress(rx);
                        CdpAction::Skip
                    }
                }
            }; // mutex dropped here

            // Now do async CDP work outside the lock
            if let CdpAction::Enrich(port, page_id) = cdp_action {
                if !try_enrich(&mut raw, port, &page_id, wv_left, wv_top).await {
                    // CDP failed (dead socket, app restart). Reconnect immediately
                    // rather than deferring to background — the caller is already
                    // waiting for hierarchy data.
                    if let Some(new_state) = setup_cdp(&self.device_serial).await {
                        try_enrich(&mut raw, new_state.port, &new_state.page_id, wv_left, wv_top).await;
                        let mut cdp = self.cdp.lock().expect("cdp mutex poisoned");
                        *cdp = CdpLifecycle::Ready(new_state);
                    } else {
                        let mut cdp = self.cdp.lock().expect("cdp mutex poisoned");
                        *cdp = CdpLifecycle::Failed;
                    }
                }
            }
        }

        // Reconstruct the wrapper with the enriched tree for parse_hierarchy
        let original: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let mut response = serde_json::json!({ "tree": raw });
        if let Some(obj) = original.as_object() {
            for key in ["keyboard_height", "safe_area_top", "safe_area_bottom", "cutouts", "rounded_corners"] {
                if let Some(val) = obj.get(key) {
                    response[key] = val.clone();
                }
            }
        }
        let enriched_str = serde_json::to_string(&response)
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
        assert_eq!(element.0.element_type, "FrameLayout");
        assert_eq!(element.0.accessibility_label.as_deref(), Some("root"));
        assert_eq!(element.0.children.len(), 1);

        let btn = &element.0.children[0];
        assert_eq!(btn.element_type, "Button");
        assert_eq!(btn.text.as_deref(), Some("Login"));
        assert_eq!(btn.accessibility_label.as_deref(), Some("login_btn"));
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
        assert_eq!(element.0.element_type, "View");
        assert!(element.0.children.is_empty());
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
        assert_eq!(element.0.element_type, "DecorView");
        assert_eq!(element.0.children.len(), 1);
        assert_eq!(element.0.children[0].element_type, "LinearLayout");
        assert_eq!(element.0.children[0].children.len(), 1);
        assert_eq!(element.0.children[0].children[0].element_type, "TextView");
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
    // 10. ADB command construction for launch_app
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
