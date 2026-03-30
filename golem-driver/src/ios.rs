use crate::common::{
    build_alert_body, build_backspace_body, build_long_press_body, build_swipe_body,
    build_tap_body, build_type_body, find_alert, parse_hierarchy, CompanionClient, SwipeRequest,
};
use crate::{Direction, PlatformDriver, ScreenshotResult};
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
}

/// Convert a `Direction` to swipe coordinate deltas.
///
/// Uses a standard iPhone screen size (390x844) with the center as
/// the origin point and a 200pt gesture distance.
fn direction_to_swipe_coords(direction: Direction) -> SwipeRequest {
    let center_x: i32 = 195;
    let center_y: i32 = 422;
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

impl IosDriver {
    /// Create a new iOS driver targeting the companion server at the given port.
    pub fn new(device_id: String, bundle_id: String, port: u16) -> Self {
        let mut client = CompanionClient::new(port);
        client.default_query = format!("bundle_id={bundle_id}");
        Self {
            client,
            device_id,
            bundle_id,
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

#[async_trait]
impl PlatformDriver for IosDriver {
    async fn get_hierarchy(&self) -> Result<Element> {
        let text = self.client.get_text("/hierarchy").await?;
        parse_hierarchy(&text)
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
        let body = serde_json::json!({ "bundle_id": bundle_id }).to_string();
        self.client.post_json("/launch", &body).await?;
        Ok(())
    }

    async fn stop_app(&self, bundle_id: &str) -> Result<()> {
        let body = serde_json::json!({ "bundle_id": bundle_id }).to_string();
        self.client.post_json("/stop", &body).await?;
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
        Ok(()) // Not applicable to iOS
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
        assert_eq!(element.element_type, "View");
        assert_eq!(element.accessibility_id.as_deref(), Some("root"));
        assert_eq!(element.children.len(), 1);

        let btn = &element.children[0];
        assert_eq!(btn.element_type, "Button");
        assert_eq!(btn.text.as_deref(), Some("Login"));
        assert_eq!(btn.accessibility_id.as_deref(), Some("login_btn"));
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
        assert_eq!(element.element_type, "View");
        assert!(element.children.is_empty());
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
        assert_eq!(element.element_type, "Window");
        assert_eq!(element.children.len(), 1);
        assert_eq!(element.children[0].element_type, "View");
        assert_eq!(element.children[0].children.len(), 1);
        assert_eq!(element.children[0].children[0].element_type, "Label");
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
    // 7. Swipe request serialization with Direction conversion
    // -----------------------------------------------------------------------
    #[test]
    fn swipe_direction_up() {
        let req = direction_to_swipe_coords(Direction::Up);
        // Swiping up: from_y > to_y (finger moves up)
        assert!(req.from_y > req.to_y, "swiping up means from_y > to_y");
        assert_eq!(req.from_x, req.to_x, "vertical swipe keeps x constant");
    }

    #[test]
    fn swipe_direction_down() {
        let req = direction_to_swipe_coords(Direction::Down);
        // Swiping down: from_y < to_y
        assert!(req.from_y < req.to_y, "swiping down means from_y < to_y");
        assert_eq!(req.from_x, req.to_x, "vertical swipe keeps x constant");
    }

    #[test]
    fn swipe_direction_left() {
        let req = direction_to_swipe_coords(Direction::Left);
        // Swiping left: from_x > to_x
        assert!(
            req.from_x > req.to_x,
            "swiping left means from_x > to_x"
        );
        assert_eq!(req.from_y, req.to_y, "horizontal swipe keeps y constant");
    }

    #[test]
    fn swipe_direction_right() {
        let req = direction_to_swipe_coords(Direction::Right);
        // Swiping right: from_x < to_x
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
    // 10. Parse alert response
    // -----------------------------------------------------------------------
    #[test]
    fn parse_alert_response_with_alert() {
        let json = r#"{
            "alert": {
                "element_type": "Alert",
                "text": "Are you sure?",
                "id": null,
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": true,
                "focused": false,
                "bounds": { "x": 50, "y": 300, "width": 290, "height": 150 },
                "children": []
            }
        }"#;

        let alert = parse_alert_response(json).expect("should parse");
        assert!(alert.is_some());
        let el = alert.expect("alert present");
        assert_eq!(el.element_type, "Alert");
        assert_eq!(el.text.as_deref(), Some("Are you sure?"));
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
    // Additional: backspace request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn backspace_request_serialization() {
        let body = build_backspace_body(5).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["count"], 5);
    }
}
