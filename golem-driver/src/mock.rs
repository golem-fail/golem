//! Mock [`PlatformDriver`] implementation for testing.

use crate::{common, GestureFinger, PlatformDriver, ScreenshotResult};
use async_trait::async_trait;
use golem_element::Element;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Mock driver for testing — records calls and returns configured responses
pub struct MockPlatformDriver {
    /// The hierarchy to return from get_hierarchy()
    pub hierarchy: Mutex<Element>,
    /// Record of all method calls (method_name, args)
    pub calls: Mutex<Vec<(String, Vec<String>)>>,
    /// Simulated soft-keyboard height — non-zero means "keyboard up".
    /// Cleared when `hide_keyboard()` is called, mirroring real driver
    /// behaviour. Mutate via `set_keyboard_height` only.
    keyboard_height: Mutex<i32>,
    /// Per-method injected errors. Keyed by the canonical trait method
    /// name; when present, that method returns `Err(anyhow!(message))`
    /// before doing anything else. Empty by default (no errors).
    errors: Mutex<HashMap<String, String>>,
    /// Queue of hierarchies for `get_hierarchy` to pop FIFO. When empty,
    /// `get_hierarchy` falls back to the steady `hierarchy` field, so
    /// single-hierarchy tests are unaffected. Empty by default.
    hierarchy_queue: Mutex<VecDeque<Element>>,
    /// Warning string `launch_app` returns as its `Some(_)` warning.
    /// `None` by default (launch returns `Ok(None)`).
    launch_warning: Mutex<Option<String>>,
    /// Path `stop_recording` returns. Defaults to the historical
    /// `"mock_recording.mp4"` so existing tests are unchanged.
    recording_path: Mutex<String>,
    /// Value `type_text`/`backspace` return as their post-mutation check.
    /// `None` by default (no verify signal, matching a driver that didn't
    /// run the check); set to `Some(true)` to simulate an un-verified
    /// mutation (slow IME) or `Some(false)` a verified one.
    type_verify: Mutex<Option<bool>>,
}

/// Map a caller-supplied method name to the canonical trait method name
/// used as the error-map key. Accepts both the real trait method names
/// and the conventional shorthands (`clear_data`, `swipe`).
fn canonical_method_name(method: &str) -> &str {
    match method {
        "clear_data" => "clear_app_data",
        "swipe" => "swipe_coords",
        other => other,
    }
}

impl MockPlatformDriver {
    /// Create a mock driver that starts with `hierarchy` as its steady-state
    /// tree (returned by `get_hierarchy` once the `hierarchy_queue` is
    /// empty) and no injected errors, keyboard height, or call history.
    pub fn new(hierarchy: Element) -> Self {
        Self {
            hierarchy: Mutex::new(hierarchy),
            calls: Mutex::new(Vec::new()),
            keyboard_height: Mutex::new(0),
            errors: Mutex::new(HashMap::new()),
            hierarchy_queue: Mutex::new(VecDeque::new()),
            launch_warning: Mutex::new(None),
            recording_path: Mutex::new("mock_recording.mp4".to_string()),
            type_verify: Mutex::new(None),
        }
    }

    pub fn set_hierarchy(&self, hierarchy: Element) {
        *self.hierarchy.lock().expect("lock poisoned") = hierarchy;
    }

    /// Set the simulated keyboard height. Zero = hidden.
    pub fn set_keyboard_height(&self, height: i32) {
        *self.keyboard_height.lock().expect("lock poisoned") = height;
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

    /// Inject an error: the named trait method returns `Err(anyhow!(message))`.
    /// Accepts the real trait method names plus the shorthands
    /// `clear_data` (→ `clear_app_data`) and `swipe` (→ `swipe_coords`).
    pub fn set_error(&self, method: &str, message: &str) {
        self.errors.lock().expect("lock poisoned").insert(
            canonical_method_name(method).to_string(),
            message.to_string(),
        );
    }

    /// Clear a previously-injected error for the named method.
    pub fn clear_error(&self, method: &str) {
        self.errors
            .lock()
            .expect("lock poisoned")
            .remove(canonical_method_name(method));
    }

    /// Return the injected error for `method` as an `Err`, if one is set.
    /// `method` here is always the canonical trait method name.
    fn check_error(&self, method: &str) -> anyhow::Result<()> {
        let msg = self
            .errors
            .lock()
            .expect("lock poisoned")
            .get(method)
            .cloned();
        match msg {
            Some(m) => Err(anyhow::anyhow!(m)),
            None => Ok(()),
        }
    }

    /// Enqueue a hierarchy. Successive `get_hierarchy()` calls pop the
    /// queue FIFO; once it empties, `get_hierarchy()` falls back to the
    /// steady `hierarchy` set via `new`/`set_hierarchy`.
    pub fn push_hierarchy(&self, hierarchy: Element) {
        self.hierarchy_queue
            .lock()
            .expect("lock poisoned")
            .push_back(hierarchy);
    }

    /// Set the warning string `launch_app` surfaces (as `Ok(Some(_))`).
    pub fn set_launch_warning(&self, warning: &str) {
        *self.launch_warning.lock().expect("lock poisoned") = Some(warning.to_string());
    }

    /// Set the path `stop_recording` returns.
    pub fn set_recording_path(&self, path: &str) {
        *self.recording_path.lock().expect("lock poisoned") = path.to_string();
    }

    /// Set the post-mutation check `type_text`/`backspace` return.
    pub fn set_type_verify(&self, verify: Option<bool>) {
        *self.type_verify.lock().expect("lock poisoned") = verify;
    }
}

#[async_trait]
impl PlatformDriver for MockPlatformDriver {
    async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
        self.record_call("get_hierarchy", vec![]);
        self.check_error("get_hierarchy")?;
        let meta = common::HierarchyMeta {
            keyboard_height: *self.keyboard_height.lock().expect("lock poisoned"),
            ..common::HierarchyMeta::default()
        };
        let tree = {
            let mut queue = self.hierarchy_queue.lock().expect("lock poisoned");
            match queue.pop_front() {
                Some(t) => t,
                None => self.hierarchy.lock().expect("lock poisoned").clone(),
            }
        };
        Ok((tree, meta))
    }

    async fn tap(&self, x: i32, y: i32) -> anyhow::Result<()> {
        self.record_call("tap", vec![x.to_string(), y.to_string()]);
        self.check_error("tap")?;
        Ok(())
    }

    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()> {
        self.record_call(
            "long_press",
            vec![x.to_string(), y.to_string(), duration_ms.to_string()],
        );
        Ok(())
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<Option<bool>> {
        self.record_call("type_text", vec![text.to_string()]);
        Ok(*self.type_verify.lock().expect("lock poisoned"))
    }

    async fn backspace(&self, count: u32) -> anyhow::Result<Option<bool>> {
        self.record_call("backspace", vec![count.to_string()]);
        Ok(*self.type_verify.lock().expect("lock poisoned"))
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
        self.check_error("swipe_coords")?;
        Ok(())
    }

    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> anyhow::Result<()> {
        self.record_call(
            "pinch",
            vec![
                x.to_string(),
                y.to_string(),
                format!("{scale}"),
                format!("{velocity}"),
            ],
        );
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
        // Mirror real driver behaviour: dismissing the keyboard zeroes
        // the height the next get_hierarchy() reports.
        *self.keyboard_height.lock().expect("lock poisoned") = 0;
        Ok(())
    }

    async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<Option<String>> {
        self.record_call("launch_app", vec![bundle_id.to_string()]);
        self.check_error("launch_app")?;
        Ok(self.launch_warning.lock().expect("lock poisoned").clone())
    }

    async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record_call("stop_app", vec![bundle_id.to_string()]);
        self.check_error("stop_app")?;
        Ok(())
    }

    async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record_call("clear_app_data", vec![bundle_id.to_string()]);
        self.check_error("clear_app_data")?;
        Ok(())
    }

    async fn press_button(&self, button: &str) -> anyhow::Result<()> {
        self.record_call("press_button", vec![button.to_string()]);
        self.check_error("press_button")?;
        Ok(())
    }

    async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()> {
        self.record_call("set_dark_mode", vec![enabled.to_string()]);
        self.check_error("set_dark_mode")?;
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()> {
        self.record_call("set_location", vec![lat.to_string(), lon.to_string()]);
        self.check_error("set_location")?;
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
        self.check_error("grant_permission")?;
        Ok(())
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.record_call(
            "revoke_permission",
            vec![bundle_id.to_string(), permission.to_string()],
        );
        self.check_error("revoke_permission")?;
        Ok(())
    }

    async fn start_recording(&self, name: &str) -> anyhow::Result<()> {
        self.record_call("start_recording", vec![name.to_string()]);
        self.check_error("start_recording")?;
        Ok(())
    }

    async fn stop_recording(&self) -> anyhow::Result<String> {
        self.record_call("stop_recording", vec![]);
        self.check_error("stop_recording")?;
        Ok(self.recording_path.lock().expect("lock poisoned").clone())
    }

    async fn remove_port_forwards(&self) -> anyhow::Result<()> {
        self.record_call("remove_port_forwards", vec![]);
        Ok(())
    }
}
