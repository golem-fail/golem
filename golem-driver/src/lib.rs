pub mod android;
pub mod cdp;
pub mod common;
pub mod commands;
pub mod ios;
pub mod ios_display;
pub mod webkit;

pub use common::CompanionHealth;

use async_trait::async_trait;
use golem_element::Element;
use std::sync::Mutex;

/// Direction for swipe and scroll gestures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// A single finger path in a multi-touch gesture.
#[derive(Debug, Clone)]
pub struct GestureFinger {
    pub points: Vec<(i32, i32)>,
    pub duration_ms: u64,
}

/// Result of a screenshot capture
#[derive(Debug, Clone)]
pub struct ScreenshotResult {
    pub path: String,
    pub data: Vec<u8>,
}

/// Trait defining all platform interactions (iOS/Android)
/// Implemented by IosDriver and AndroidDriver, mocked for testing
#[async_trait]
pub trait PlatformDriver: Send + Sync {
    /// Get the full element hierarchy and metadata (keyboard state, etc.)
    async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)>;

    /// Tap at specific screen coordinates
    async fn tap(&self, x: i32, y: i32) -> anyhow::Result<()>;

    /// Long press at coordinates for a duration
    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()>;

    /// Type text into the currently focused field
    async fn type_text(&self, text: &str) -> anyhow::Result<()>;

    /// Delete characters (backspace)
    async fn backspace(&self, count: u32) -> anyhow::Result<()>;

    /// Perform a swipe between specific coordinates
    async fn swipe_coords(
        &self,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
    ) -> anyhow::Result<()>;

    /// Pinch gesture at coordinates.
    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> anyhow::Result<()>;

    /// Perform a multi-touch gesture (continuous paths, arbitrary multi-touch).
    /// Each finger has its own path of points and duration.
    async fn gesture(&self, fingers: Vec<GestureFinger>) -> anyhow::Result<()>;

    /// Take a screenshot
    async fn screenshot(&self) -> anyhow::Result<ScreenshotResult>;

    /// Hide the on-screen keyboard
    async fn hide_keyboard(&self) -> anyhow::Result<()>;

    /// Launch the app with the given bundle/package ID
    async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<()>;

    /// Stop/force-kill the app
    async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()>;

    /// Clear app data/cache
    async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()>;

    /// Press a hardware button (home, back, volume_up, volume_down)
    async fn press_button(&self, button: &str) -> anyhow::Result<()>;

    /// Set device orientation
    async fn set_orientation(&self, orientation: &str) -> anyhow::Result<()>;

    /// Set dark mode
    async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()>;

    /// Mock GPS location
    async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()>;

    /// Open a URL/deep link
    async fn open_url(&self, url: &str) -> anyhow::Result<()>;

    /// Send a push notification
    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        payload: Option<&str>,
    ) -> anyhow::Result<()>;

    /// Add media to device library
    async fn add_media(&self, path: &str) -> anyhow::Result<()>;

    /// Grant app permission
    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()>;

    /// Revoke app permission
    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()>;

    /// Start screen recording
    async fn start_recording(&self, name: &str) -> anyhow::Result<()>;

    /// Stop screen recording
    async fn stop_recording(&self) -> anyhow::Result<String>;

    /// Remove adb port forwards (Android-only; no-op on iOS)
    async fn remove_port_forwards(&self) -> anyhow::Result<()>;
}

/// Mock driver for testing — records calls and returns configured responses
pub struct MockPlatformDriver {
    /// The hierarchy to return from get_hierarchy()
    pub hierarchy: Mutex<Element>,
    /// Record of all method calls (method_name, args)
    pub calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl MockPlatformDriver {
    pub fn new(hierarchy: Element) -> Self {
        Self {
            hierarchy: Mutex::new(hierarchy),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn set_hierarchy(&self, hierarchy: Element) {
        *self.hierarchy.lock().expect("lock poisoned") = hierarchy;
    }

    /// Get all recorded calls
    pub fn get_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().expect("lock poisoned").clone()
    }

    /// Clear recorded calls
    pub fn clear_calls(&self) {
        self.calls.lock().expect("lock poisoned").clear();
    }

    fn record_call(&self, method: &str, args: Vec<String>) {
        self.calls
            .lock()
            .expect("lock poisoned")
            .push((method.to_string(), args));
    }
}

#[async_trait]
impl PlatformDriver for MockPlatformDriver {
    async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
        self.record_call("get_hierarchy", vec![]);
        Ok((self.hierarchy.lock().expect("lock poisoned").clone(), common::HierarchyMeta::default()))
    }

    async fn tap(&self, x: i32, y: i32) -> anyhow::Result<()> {
        self.record_call("tap", vec![x.to_string(), y.to_string()]);
        Ok(())
    }

    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()> {
        self.record_call(
            "long_press",
            vec![x.to_string(), y.to_string(), duration_ms.to_string()],
        );
        Ok(())
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.record_call("type_text", vec![text.to_string()]);
        Ok(())
    }

    async fn backspace(&self, count: u32) -> anyhow::Result<()> {
        self.record_call("backspace", vec![count.to_string()]);
        Ok(())
    }


    async fn swipe_coords(
        &self,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
    ) -> anyhow::Result<()> {
        self.record_call(
            "swipe_coords",
            vec![
                from_x.to_string(),
                from_y.to_string(),
                to_x.to_string(),
                to_y.to_string(),
            ],
        );
        Ok(())
    }

    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> anyhow::Result<()> {
        self.record_call("pinch", vec![x.to_string(), y.to_string(), format!("{scale}"), format!("{velocity}")]);
        Ok(())
    }

    async fn gesture(&self, fingers: Vec<GestureFinger>) -> anyhow::Result<()> {
        let args: Vec<String> = fingers
            .iter()
            .map(|f| format!("{}pts@{}ms", f.points.len(), f.duration_ms))
            .collect();
        self.record_call("gesture", args);
        Ok(())
    }

    async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
        self.record_call("screenshot", vec![]);
        Ok(ScreenshotResult {
            path: "mock_screenshot.png".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47], // PNG magic bytes
        })
    }

    async fn hide_keyboard(&self) -> anyhow::Result<()> {
        self.record_call("hide_keyboard", vec![]);
        Ok(())
    }

    async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record_call("launch_app", vec![bundle_id.to_string()]);
        Ok(())
    }

    async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record_call("stop_app", vec![bundle_id.to_string()]);
        Ok(())
    }

    async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record_call("clear_app_data", vec![bundle_id.to_string()]);
        Ok(())
    }

    async fn press_button(&self, button: &str) -> anyhow::Result<()> {
        self.record_call("press_button", vec![button.to_string()]);
        Ok(())
    }

    async fn set_orientation(&self, orientation: &str) -> anyhow::Result<()> {
        self.record_call("set_orientation", vec![orientation.to_string()]);
        Ok(())
    }

    async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()> {
        self.record_call("set_dark_mode", vec![enabled.to_string()]);
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()> {
        self.record_call("set_location", vec![lat.to_string(), lon.to_string()]);
        Ok(())
    }

    async fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.record_call("open_url", vec![url.to_string()]);
        Ok(())
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        payload: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut args = vec![title.to_string(), body.to_string()];
        if let Some(p) = payload {
            args.push(p.to_string());
        }
        self.record_call("push_notification", args);
        Ok(())
    }

    async fn add_media(&self, path: &str) -> anyhow::Result<()> {
        self.record_call("add_media", vec![path.to_string()]);
        Ok(())
    }

    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.record_call(
            "grant_permission",
            vec![bundle_id.to_string(), permission.to_string()],
        );
        Ok(())
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.record_call(
            "revoke_permission",
            vec![bundle_id.to_string(), permission.to_string()],
        );
        Ok(())
    }

    async fn start_recording(&self, name: &str) -> anyhow::Result<()> {
        self.record_call("start_recording", vec![name.to_string()]);
        Ok(())
    }

    async fn stop_recording(&self) -> anyhow::Result<String> {
        self.record_call("stop_recording", vec![]);
        Ok("mock_recording.mp4".to_string())
    }

    async fn remove_port_forwards(&self) -> anyhow::Result<()> {
        self.record_call("remove_port_forwards", vec![]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::Bounds;

    fn make_element(element_type: &str, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds,
            visible_bounds: None,
            children: Vec::new(),
        }
    }

    fn default_hierarchy() -> Element {
        make_element("View", Bounds::new(0, 0, 375, 812))
    }

    #[tokio::test]
    async fn mock_records_tap_calls_with_coordinates() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        driver.tap(100, 200).await.expect("tap failed");
        driver.tap(50, 75).await.expect("tap failed");

        let calls = driver.get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "tap");
        assert_eq!(calls[0].1, vec!["100", "200"]);
        assert_eq!(calls[1].0, "tap");
        assert_eq!(calls[1].1, vec!["50", "75"]);
    }

    #[tokio::test]
    async fn mock_returns_configured_hierarchy() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let button = make_element("Button", Bounds::new(10, 10, 100, 44));
        root.children.push(button);

        let driver = MockPlatformDriver::new(root);

        let hierarchy = driver.get_hierarchy().await.expect("get_hierarchy failed");
        assert_eq!(hierarchy.0.element_type, "View");
        assert_eq!(hierarchy.0.children.len(), 1);
        assert_eq!(hierarchy.0.children[0].element_type, "Button");

        // Update hierarchy and verify it changes
        let new_root = make_element("Screen", Bounds::new(0, 0, 390, 844));
        driver.set_hierarchy(new_root);

        let updated = driver.get_hierarchy().await.expect("get_hierarchy failed");
        assert_eq!(updated.0.element_type, "Screen");
        assert!(updated.0.children.is_empty());
    }

    #[tokio::test]
    async fn mock_records_type_text_calls() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        driver
            .type_text("hello world")
            .await
            .expect("type_text failed");
        driver
            .type_text("goodbye")
            .await
            .expect("type_text failed");

        let calls = driver.get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "type_text");
        assert_eq!(calls[0].1, vec!["hello world"]);
        assert_eq!(calls[1].0, "type_text");
        assert_eq!(calls[1].1, vec!["goodbye"]);
    }

    #[test]
    fn direction_enum_equality() {
        assert_eq!(Direction::Up, Direction::Up);
        assert_eq!(Direction::Down, Direction::Down);
        assert_eq!(Direction::Left, Direction::Left);
        assert_eq!(Direction::Right, Direction::Right);

        assert_ne!(Direction::Up, Direction::Down);
        assert_ne!(Direction::Left, Direction::Right);
        assert_ne!(Direction::Up, Direction::Left);
    }
}
