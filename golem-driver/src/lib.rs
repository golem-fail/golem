pub mod android;
pub mod cdp;
pub mod commands;
pub mod common;
pub mod ime;
pub mod ios;

pub mod ios_display;
pub mod webkit;

pub use common::CompanionHealth;

use async_trait::async_trait;
use golem_element::Element;
use std::collections::{HashMap, VecDeque};
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

    /// Launch the app with the given bundle/package ID. Returns an
    /// optional warning string when the launch succeeded but the
    /// platform layer wants the next step to know about a soft race
    /// (e.g. iOS settle-probe timed out, app is foregrounded but DOM
    /// may not be painted yet). Callers should surface the warning
    /// as a `DriverWarning` substep — the launch itself is still ok.
    async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<Option<String>>;

    /// Stop/force-kill the app
    async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()>;

    /// Clear app data/cache
    async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()>;

    /// Press a hardware button (home, back, volume_up, volume_down)
    async fn press_button(&self, button: &str) -> anyhow::Result<()>;

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

    /// Poke a no-op interaction on the test app to give XCTest a chance
    /// to invoke its UI-interruption-monitor handler (iOS only). The
    /// monitor auto-dismisses OS-owned dialogs (deep-link "Open in
    /// <App>?" confirms, permission prompts) without cross-app XCUI
    /// queries — see the roadmap entry on UIInterruptionMonitor for
    /// why we can't query SpringBoard directly. Default: no-op (the
    /// concept is iOS-XCUITest-specific).
    async fn poke_for_system_alert(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Prepare to type `text` into the next-focused field. Called by the
    /// runner BEFORE the focus tap, so any input-method switch is in
    /// place when the field gains focus. Android uses this to lazily
    /// activate its Unicode IME when `text` contains non-ASCII (the
    /// `input text` shell path is ASCII-only); the activation must
    /// precede the focus tap so the field's input connection binds to
    /// the golem IME. Default: no-op (iOS `typeText` is Unicode-capable;
    /// mock drivers need nothing).
    async fn prepare_type(&self, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    /// Set a per-HTTP-request timeout on the driver's companion client.
    /// Called by the runner before each step so a wedged companion fails
    /// fast at the connection layer rather than burning the outer
    /// `tokio::time::timeout` budget. Default: no-op (mock drivers).
    fn set_request_timeout(&self, _timeout: std::time::Duration) {}

    /// Wait for the UI to finish rendering after a launch. Polls
    /// `get_hierarchy()` and returns once the tree is non-empty AND
    /// stable across two consecutive polls — i.e. the first interactive
    /// screen has rendered.
    ///
    /// This is the post-launch settle gate. Without it, the first action
    /// after `launch_app` races against the OS finishing the spawn /
    /// the app drawing its first frame / the accessibility tree
    /// populating. See the `Audit existing flake roadmap entries` task
    /// — three e2e flakes (cold-boot iOS Submit timeout, tap_roundtrip
    /// "+", WebView first action) all share this single root cause.
    ///
    /// Settle conditions:
    /// - tree node count ≥ [`AWAIT_FIRST_FRAME_MIN_NODES`] (filters
    ///   out splash screens / pre-render states)
    /// - same count observed across 2 consecutive polls (UI has stopped
    ///   re-rendering)
    ///
    /// Returns `Ok(None)` on settle OR on deadline. Returns `Ok(Some(warning))`
    /// when the gate proceeded at its deadline with a WebView still unhydrated
    /// — a launch warning the caller surfaces instead of a silent stall. Never
    /// fails launch: if the gate is wrong, the downstream action's own timeout
    /// catches genuinely broken cases.
    async fn await_first_frame(&self) -> anyhow::Result<Option<String>> {
        await_first_frame_default(self).await
    }
}

/// Minimum node count to consider a tree to represent a real first screen.
/// Set above the typical splash / status-bar-only state — empirical
/// observation on iOS sims shows pre-render snapshots in the 5-15 range
/// (status bar, navigation chrome, transition overlays). Real first
/// screens land at 30-100+ nodes. 20 is a conservative cut.
pub const AWAIT_FIRST_FRAME_MIN_NODES: usize = 20;

/// Poll interval for the settle gate. 200ms balances responsiveness
/// (settle in 400ms typical) against companion load (5 polls/sec).
pub const AWAIT_FIRST_FRAME_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(200);

/// Hard deadline for native screens. Beyond this we proceed anyway — the
/// downstream action's own timeout will catch genuinely broken UI states.
pub const AWAIT_FIRST_FRAME_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Extended deadline used once a WebView node is detected. The native
/// accessibility tree settles long before the webview DOM hydrates (CDP /
/// WebKit Inspector enrichment), so a webview-bearing launch is given more
/// budget to render its first page before the gate gives up. A page that
/// never hydrates within this window proceeds with a launch warning.
pub const AWAIT_FIRST_FRAME_WEBVIEW_DEADLINE: std::time::Duration =
    std::time::Duration::from_secs(30);

/// Minimum WebView-subtree node count to consider the page hydrated. An
/// unrendered WebView is just the host node (and at most an empty document
/// shell); a real first screen has many more DOM nodes. Below this the gate
/// treats the webview as not-yet-ready and keeps waiting. Heuristic — the
/// extended deadline + launch warning is the backstop if a genuinely tiny
/// page never crosses it.
pub const AWAIT_FIRST_FRAME_WEBVIEW_MIN_NODES: usize = 10;

/// Number of consecutive stable polls required to declare settle.
const STABLE_POLLS_REQUIRED: u32 = 2;

/// Default settle implementation — shared between iOS and Android since
/// both use the same `get_hierarchy` companion endpoint. Drivers can
/// override `await_first_frame` to add platform-specific signals (e.g.
/// WebKit Inspector readiness on iOS WebView screens) but for native
/// flows this is sufficient.
async fn await_first_frame_default(
    driver: &(impl PlatformDriver + ?Sized),
) -> anyhow::Result<Option<String>> {
    // Use `tokio::time::Instant` (not `std::time`) so this respects
    // tokio's paused-time test mode. Otherwise unit tests that simulate
    // the deadline would burn real wall-clock.
    let start = tokio::time::Instant::now();
    let mut prev_count: usize = 0;
    let mut stable_polls: u32 = 0;
    // Whether any poll has seen a WebView node, and that webview's most
    // recent subtree size. Drives the extended deadline + the not-ready
    // warning. The native a11y tree settles long before the webview DOM
    // hydrates, so a webview launch needs both a longer budget and an
    // explicit hydration check — not just native-frame stability.
    let mut webview_seen = false;
    let mut last_webview_count: usize = 0;
    loop {
        // The deadline extends once a WebView is detected: webview launches
        // legitimately take longer to render their first page than native.
        let deadline = if webview_seen {
            AWAIT_FIRST_FRAME_WEBVIEW_DEADLINE
        } else {
            AWAIT_FIRST_FRAME_DEADLINE
        };
        if start.elapsed() >= deadline {
            // A webview that never hydrated within the window: proceed (the
            // downstream action still gets its own timeout) but surface a
            // warning so a sparse-tree start is a labelled diagnostic, not a
            // mystery 120s scroll thrash.
            if webview_seen && last_webview_count < AWAIT_FIRST_FRAME_WEBVIEW_MIN_NODES {
                let warning = format!(
                    "webview DOM not ready after {:?} ({last_webview_count} nodes) — first action may run against an unrendered page",
                    start.elapsed()
                );
                if golem_common::is_debug() {
                    eprintln!("  [launch] {warning}");
                }
                return Ok(Some(warning));
            }
            if golem_common::is_debug() {
                eprintln!(
                    "  [launch] settle deadline reached after {:?}, last seen {prev_count} nodes — proceeding anyway",
                    start.elapsed()
                );
            }
            return Ok(None);
        }
        let (count, webview_count) = match driver.get_hierarchy().await {
            Ok((tree, _)) => (tree.node_count(), tree.webview_subtree_count()),
            // Tree-fetch errors mid-launch are common (companion port
            // briefly unresponsive). Treat as 0 nodes and keep polling
            // — bubbling the error would fail launch entirely.
            Err(_) => (0, None),
        };
        if let Some(wc) = webview_count {
            webview_seen = true;
            last_webview_count = wc;
        }
        // A webview screen is "ready" only once its DOM subtree is non-trivial;
        // a native screen (no webview) is always ready on this axis.
        let webview_ready = match webview_count {
            Some(wc) => wc >= AWAIT_FIRST_FRAME_WEBVIEW_MIN_NODES,
            None => true,
        };
        if count >= AWAIT_FIRST_FRAME_MIN_NODES && count == prev_count && webview_ready {
            stable_polls += 1;
            if stable_polls >= STABLE_POLLS_REQUIRED {
                if golem_common::is_debug() {
                    eprintln!(
                        "  [launch] UI settled in {:?} ({count} nodes)",
                        start.elapsed()
                    );
                }
                return Ok(None);
            }
        } else {
            stable_polls = 0;
        }
        prev_count = count;
        tokio::time::sleep(AWAIT_FIRST_FRAME_POLL_INTERVAL).await;
    }
}

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
    pub fn new(hierarchy: Element) -> Self {
        Self {
            hierarchy: Mutex::new(hierarchy),
            calls: Mutex::new(Vec::new()),
            keyboard_height: Mutex::new(0),
            errors: Mutex::new(HashMap::new()),
            hierarchy_queue: Mutex::new(VecDeque::new()),
            launch_warning: Mutex::new(None),
            recording_path: Mutex::new("mock_recording.mp4".to_string()),
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
            hit_points: vec![],
            drawing_order: None,
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
        driver.type_text("goodbye").await.expect("type_text failed");

        let calls = driver.get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "type_text");
        assert_eq!(calls[0].1, vec!["hello world"]);
        assert_eq!(calls[1].0, "type_text");
        assert_eq!(calls[1].1, vec!["goodbye"]);
    }

    // ── MockPlatformDriver additive capabilities ───────────────────

    #[tokio::test]
    async fn mock_set_error_makes_method_return_err() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        driver.set_error("tap", "boom");
        let err = driver.tap(1, 2).await.expect_err("tap SHALL return Err");
        assert_eq!(err.to_string(), "boom");

        // Other methods stay Ok.
        driver
            .launch_app("com.x")
            .await
            .expect("launch SHALL be Ok");
    }

    #[tokio::test]
    async fn mock_clear_error_restores_ok() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        driver.set_error("get_hierarchy", "down");
        driver
            .get_hierarchy()
            .await
            .expect_err("get_hierarchy SHALL fail while error set");

        driver.clear_error("get_hierarchy");
        driver
            .get_hierarchy()
            .await
            .expect("get_hierarchy SHALL succeed after clear_error");
    }

    #[tokio::test]
    async fn mock_error_accepts_shorthand_method_names() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        driver.set_error("clear_data", "no");
        driver
            .clear_app_data("com.x")
            .await
            .expect_err("clear_data shorthand SHALL inject into clear_app_data");

        driver.set_error("swipe", "no");
        driver
            .swipe_coords(0, 0, 1, 1)
            .await
            .expect_err("swipe shorthand SHALL inject into swipe_coords");
    }

    #[tokio::test]
    async fn mock_push_hierarchy_pops_fifo_then_falls_back() {
        let steady = make_element("Steady", Bounds::new(0, 0, 10, 10));
        let driver = MockPlatformDriver::new(steady);

        driver.push_hierarchy(make_element("First", Bounds::new(0, 0, 1, 1)));
        driver.push_hierarchy(make_element("Second", Bounds::new(0, 0, 1, 1)));

        let a = driver.get_hierarchy().await.expect("get_hierarchy");
        assert_eq!(a.0.element_type, "First");
        let b = driver.get_hierarchy().await.expect("get_hierarchy");
        assert_eq!(b.0.element_type, "Second");

        // Queue drained → falls back to steady hierarchy.
        let c = driver.get_hierarchy().await.expect("get_hierarchy");
        assert_eq!(c.0.element_type, "Steady");
        let d = driver.get_hierarchy().await.expect("get_hierarchy");
        assert_eq!(d.0.element_type, "Steady");
    }

    #[tokio::test]
    async fn mock_set_launch_warning_surfaces_in_launch_app() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        // Default: no warning.
        assert_eq!(driver.launch_app("com.x").await.expect("launch"), None);

        driver.set_launch_warning("settle-probe timed out");
        assert_eq!(
            driver.launch_app("com.x").await.expect("launch"),
            Some("settle-probe timed out".to_string())
        );
    }

    #[tokio::test]
    async fn mock_set_recording_path_overrides_default() {
        let driver = MockPlatformDriver::new(default_hierarchy());

        // Default path preserved for existing tests.
        assert_eq!(
            driver.stop_recording().await.expect("stop"),
            "mock_recording.mp4"
        );

        driver.set_recording_path("/tmp/custom.mp4");
        assert_eq!(
            driver.stop_recording().await.expect("stop"),
            "/tmp/custom.mp4"
        );
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

    // ── await_first_frame settle gate ──────────────────────────────
    //
    // Drives the gate against synthetic hierarchies whose node counts
    // change over time so we can exercise the splash → first-screen
    // settle path without a real device.

    /// Build a hierarchy with `child_count` direct children (so total
    /// `node_count` == `child_count + 1`).
    fn tree_with_children(child_count: usize) -> Element {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children = (0..child_count)
            .map(|_| make_element("Button", Bounds::new(0, 0, 10, 10)))
            .collect();
        root
    }

    /// Mock that returns a different hierarchy on each `get_hierarchy`
    /// call, drawn from a queue. After the queue empties, every
    /// subsequent call returns the last element. Used to drive the
    /// settle algorithm through scripted node-count progressions.
    struct SequencedMock {
        queue: Mutex<Vec<Element>>,
    }

    impl SequencedMock {
        fn new(progression: Vec<usize>) -> Self {
            // Reverse so we can `pop()` cheaply from the end as the
            // queue advances.
            let mut frames: Vec<Element> =
                progression.into_iter().map(tree_with_children).collect();
            frames.reverse();
            Self {
                queue: Mutex::new(frames),
            }
        }
    }

    #[async_trait]
    impl PlatformDriver for SequencedMock {
        async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
            let tree = {
                let mut q = self.queue.lock().expect("lock poisoned");
                if q.len() > 1 {
                    q.pop().expect("non-empty")
                } else {
                    // Last element: return without removing so subsequent
                    // calls keep returning the steady-state tree.
                    q.last().cloned().unwrap_or_else(|| tree_with_children(0))
                }
            };
            Ok((tree, common::HierarchyMeta::default()))
        }
        async fn tap(&self, _x: i32, _y: i32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn long_press(&self, _x: i32, _y: i32, _d: u64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn type_text(&self, _t: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn backspace(&self, _c: u32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn swipe_coords(&self, _: i32, _: i32, _: i32, _: i32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn pinch(&self, _x: i32, _y: i32, _s: f64, _v: f64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn gesture(&self, _f: Vec<GestureFinger>) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
            unimplemented!()
        }
        async fn hide_keyboard(&self) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn launch_app(&self, _b: &str) -> anyhow::Result<Option<String>> {
            unimplemented!()
        }
        async fn stop_app(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn clear_app_data(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn press_button(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn set_dark_mode(&self, _e: bool) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn set_location(&self, _: f64, _: f64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn open_url(&self, _u: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn push_notification(&self, _: &str, _: &str, _: Option<&str>) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn add_media(&self, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn grant_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn revoke_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn start_recording(&self, _n: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn stop_recording(&self) -> anyhow::Result<String> {
            unimplemented!()
        }
        async fn remove_port_forwards(&self) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    // Helper picks a child count safely above the MIN_NODES threshold so
    // the gate accepts the resulting tree as a real first screen.
    fn above_threshold() -> usize {
        AWAIT_FIRST_FRAME_MIN_NODES + 10
    }

    fn below_threshold() -> usize {
        AWAIT_FIRST_FRAME_MIN_NODES.saturating_sub(5)
    }

    /// Build a tree with `native` plain children PLUS a WebView node whose DOM
    /// subtree has `dom` descendants. Total native count = native + 1 (root)
    /// + 1 (webview node); webview_subtree_count = 1 + dom.
    fn tree_with_webview(native: usize, dom: usize) -> Element {
        let mut root = tree_with_children(native);
        let mut wv = make_element("web_view", Bounds::new(0, 0, 300, 600));
        wv.children = (0..dom)
            .map(|_| make_element("div", Bounds::new(0, 0, 10, 10)))
            .collect();
        root.children.push(wv);
        root
    }

    /// Like `SequencedMock` but each frame is a webview tree: the progression
    /// gives the DOM-descendant count per poll (native chrome held above the
    /// node threshold so only webview hydration is under test).
    struct WebviewSequencedMock {
        queue: Mutex<Vec<Element>>,
    }

    impl WebviewSequencedMock {
        fn new(dom_progression: Vec<usize>) -> Self {
            let mut frames: Vec<Element> = dom_progression
                .into_iter()
                .map(|dom| tree_with_webview(above_threshold(), dom))
                .collect();
            frames.reverse();
            Self {
                queue: Mutex::new(frames),
            }
        }
    }

    #[async_trait]
    impl PlatformDriver for WebviewSequencedMock {
        async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
            let tree = {
                let mut q = self.queue.lock().expect("lock poisoned");
                if q.len() > 1 {
                    q.pop().expect("non-empty")
                } else {
                    q.last()
                        .cloned()
                        .unwrap_or_else(|| tree_with_webview(above_threshold(), 0))
                }
            };
            Ok((tree, common::HierarchyMeta::default()))
        }
        async fn tap(&self, _x: i32, _y: i32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn long_press(&self, _x: i32, _y: i32, _d: u64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn type_text(&self, _t: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn backspace(&self, _c: u32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn swipe_coords(&self, _: i32, _: i32, _: i32, _: i32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn pinch(&self, _x: i32, _y: i32, _s: f64, _v: f64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn gesture(&self, _f: Vec<GestureFinger>) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
            unimplemented!()
        }
        async fn hide_keyboard(&self) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn launch_app(&self, _b: &str) -> anyhow::Result<Option<String>> {
            unimplemented!()
        }
        async fn stop_app(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn clear_app_data(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn press_button(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn set_dark_mode(&self, _e: bool) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn set_location(&self, _: f64, _: f64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn open_url(&self, _u: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn push_notification(&self, _: &str, _: &str, _: Option<&str>) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn add_media(&self, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn grant_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn revoke_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn start_recording(&self, _n: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn stop_recording(&self) -> anyhow::Result<String> {
            unimplemented!()
        }
        async fn remove_port_forwards(&self) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_returns_when_count_stabilises_above_threshold() {
        // Splash (0) → small below-threshold render → real screen (high
        // count) → stable. Expect settle on the second stable poll.
        let high = above_threshold();
        let driver = SequencedMock::new(vec![0, below_threshold(), high, high, high]);
        let start = tokio::time::Instant::now();
        driver.await_first_frame().await.unwrap();
        // Each poll is 200ms; we need at least 4 polls (0 → below →
        // high → high stable).
        assert!(
            start.elapsed() >= AWAIT_FIRST_FRAME_POLL_INTERVAL * 3,
            "settle SHALL wait through the splash + settle window: {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_ignores_stable_below_threshold() {
        // Tree stays below the threshold for many polls — settle SHALL
        // keep waiting. Then jumps above + stable.
        let low = below_threshold();
        let high = above_threshold();
        let driver = SequencedMock::new(vec![low, low, low, low, high, high]);
        let start = tokio::time::Instant::now();
        driver.await_first_frame().await.unwrap();
        // 5 transitions before stability means 5 polls minimum.
        assert!(start.elapsed() >= AWAIT_FIRST_FRAME_POLL_INTERVAL * 4);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_returns_at_deadline_without_settle() {
        // Tree drains to a steady value, so settle DOES eventually fire.
        // We assert only that we returned within the deadline window.
        let high = above_threshold();
        let driver = SequencedMock::new(vec![high, high + 1, high + 2, high + 3, high + 4]);
        let start = tokio::time::Instant::now();
        driver.await_first_frame().await.unwrap();
        assert!(start.elapsed() <= AWAIT_FIRST_FRAME_DEADLINE + AWAIT_FIRST_FRAME_POLL_INTERVAL);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_does_not_settle_below_min_nodes() {
        // Tree stabilises below the threshold — settle SHALL wait until
        // the deadline (and then return Ok anyway).
        let driver = SequencedMock::new(vec![below_threshold()]);
        let start = tokio::time::Instant::now();
        driver.await_first_frame().await.unwrap();
        assert!(
            start.elapsed() >= AWAIT_FIRST_FRAME_DEADLINE,
            "below-threshold tree SHALL wait until deadline: {:?}",
            start.elapsed()
        );
    }

    // ── settle gate: webview hydration ─────────────────────────────

    // A webview whose DOM hydrates (sparse → above the webview threshold,
    // then stable) settles cleanly with no warning, even though the native
    // node count was above-threshold the whole time. Guards the scenario
    // where native chrome alone would have settled the gate too early.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_waits_for_webview_dom_then_settles() {
        let ready = AWAIT_FIRST_FRAME_WEBVIEW_MIN_NODES + 20;
        // DOM: empty for several polls, then hydrates and holds.
        let driver = WebviewSequencedMock::new(vec![0, 0, 0, ready, ready, ready]);
        let start = tokio::time::Instant::now();
        let warning = driver.await_first_frame().await.unwrap();
        assert!(
            warning.is_none(),
            "a hydrated webview SHALL not warn: {warning:?}"
        );
        assert!(
            start.elapsed() >= AWAIT_FIRST_FRAME_POLL_INTERVAL * 3,
            "settle SHALL wait through the unhydrated polls: {:?}",
            start.elapsed()
        );
        assert!(
            start.elapsed() < AWAIT_FIRST_FRAME_WEBVIEW_DEADLINE,
            "a webview that hydrates SHALL settle before the extended deadline: {:?}",
            start.elapsed()
        );
    }

    // A native-stable tree with a webview that never hydrates SHALL NOT settle
    // at the 10s native deadline — it waits for the extended webview deadline,
    // then proceeds with a "webview DOM not ready" warning.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_extends_deadline_and_warns_for_sparse_webview() {
        // DOM stays empty (below the webview threshold) forever.
        let driver = WebviewSequencedMock::new(vec![0]);
        let start = tokio::time::Instant::now();
        let warning = driver.await_first_frame().await.unwrap();
        assert!(
            start.elapsed() >= AWAIT_FIRST_FRAME_WEBVIEW_DEADLINE,
            "a sparse webview SHALL wait the extended deadline, not the 10s native one: {:?}",
            start.elapsed()
        );
        let warning = warning.expect("a never-hydrated webview SHALL surface a launch warning");
        assert!(
            warning.contains("webview DOM not ready"),
            "warning SHALL name the cause: {warning}"
        );
    }

    // ── settle gate: error + reset paths ───────────────────────────

    /// Mock whose `get_hierarchy` always errors, exercising the
    /// `Err(_) => 0` branch in the settle loop. A driver that can never
    /// produce a tree SHALL never settle and SHALL bail at the deadline
    /// (returning Ok, not surfacing the error).
    struct ErroringMock;

    #[async_trait]
    impl PlatformDriver for ErroringMock {
        async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
            anyhow::bail!("companion port unresponsive")
        }
        async fn tap(&self, _x: i32, _y: i32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn long_press(&self, _x: i32, _y: i32, _d: u64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn type_text(&self, _t: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn backspace(&self, _c: u32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn swipe_coords(&self, _: i32, _: i32, _: i32, _: i32) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn pinch(&self, _x: i32, _y: i32, _s: f64, _v: f64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn gesture(&self, _f: Vec<GestureFinger>) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
            unimplemented!()
        }
        async fn hide_keyboard(&self) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn launch_app(&self, _b: &str) -> anyhow::Result<Option<String>> {
            unimplemented!()
        }
        async fn stop_app(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn clear_app_data(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn press_button(&self, _b: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn set_dark_mode(&self, _e: bool) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn set_location(&self, _: f64, _: f64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn open_url(&self, _u: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn push_notification(&self, _: &str, _: &str, _: Option<&str>) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn add_media(&self, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn grant_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn revoke_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn start_recording(&self, _n: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn stop_recording(&self) -> anyhow::Result<String> {
            unimplemented!()
        }
        async fn remove_port_forwards(&self) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    // 1. A perpetually-erroring hierarchy fetch is treated as 0 nodes and
    //    never settles; the gate SHALL still return Ok at the deadline.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_treats_hierarchy_error_as_zero_nodes() {
        let driver = ErroringMock;
        let start = tokio::time::Instant::now();
        driver
            .await_first_frame()
            .await
            .expect("settle SHALL return Ok even when hierarchy always errors");
        assert!(
            start.elapsed() >= AWAIT_FIRST_FRAME_DEADLINE,
            "erroring tree SHALL wait until the deadline: {:?}",
            start.elapsed()
        );
    }

    // 2. A single stable poll above threshold is NOT enough: when the count
    //    changes after one match, the stable-poll counter SHALL reset and
    //    settle SHALL only fire once two CONSECUTIVE equal polls land.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn await_first_frame_resets_stable_counter_on_count_change() {
        let high = above_threshold();
        // high, high (1 stable match) → high+1 (reset) → high+1, high+1
        // (settle on the 2nd consecutive match of the new value).
        let driver = SequencedMock::new(vec![high, high, high + 1, high + 1, high + 1]);
        let start = tokio::time::Instant::now();
        driver
            .await_first_frame()
            .await
            .expect("settle SHALL eventually fire");
        // The first stable run is broken by the high+1 change, so settle
        // cannot have fired before the 4th poll (>= 3 intervals elapsed).
        assert!(
            start.elapsed() >= AWAIT_FIRST_FRAME_POLL_INTERVAL * 3,
            "a changed count SHALL reset the stable counter: {:?}",
            start.elapsed()
        );
    }

    // ── MockPlatformDriver: call recording for every method ────────

    // 1. long_press records x, y and duration in order.
    #[tokio::test]
    async fn mock_records_long_press_with_duration() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver
            .long_press(12, 34, 750)
            .await
            .expect("long_press failed");
        let calls = driver.get_calls();
        assert_eq!(calls.len(), 1, "exactly one call SHALL be recorded");
        assert_eq!(calls[0].0, "long_press");
        assert_eq!(calls[0].1, vec!["12", "34", "750"]);
    }

    // 2. backspace records its count.
    #[tokio::test]
    async fn mock_records_backspace_count() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.backspace(5).await.expect("backspace failed");
        let calls = driver.get_calls();
        assert_eq!(calls[0].0, "backspace");
        assert_eq!(calls[0].1, vec!["5"], "backspace SHALL record its count");
    }

    // 3. swipe_coords records all four coordinates in from→to order.
    #[tokio::test]
    async fn mock_records_swipe_coords() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.swipe_coords(1, 2, 3, 4).await.expect("swipe failed");
        let calls = driver.get_calls();
        assert_eq!(calls[0].0, "swipe_coords");
        assert_eq!(calls[0].1, vec!["1", "2", "3", "4"]);
    }

    // 4. pinch records coordinates plus scale and velocity as floats.
    #[tokio::test]
    async fn mock_records_pinch_scale_and_velocity() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.pinch(50, 60, 2.0, 1.5).await.expect("pinch failed");
        let calls = driver.get_calls();
        assert_eq!(calls[0].0, "pinch");
        assert_eq!(calls[0].1, vec!["50", "60", "2", "1.5"]);
    }

    // 5. gesture summarises each finger as "<n>pts@<ms>ms", one arg per
    //    finger, preserving order.
    #[tokio::test]
    async fn mock_records_gesture_finger_summary() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let fingers = vec![
            GestureFinger {
                points: vec![(0, 0), (1, 1), (2, 2)],
                duration_ms: 300,
            },
            GestureFinger {
                points: vec![(9, 9)],
                duration_ms: 100,
            },
        ];
        driver.gesture(fingers).await.expect("gesture failed");
        let calls = driver.get_calls();
        assert_eq!(calls[0].0, "gesture");
        assert_eq!(calls[0].1, vec!["3pts@300ms", "1pts@100ms"]);
    }

    // 6. screenshot records exactly one no-arg "screenshot" call.
    #[tokio::test]
    async fn mock_screenshot_records_noarg_call() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.screenshot().await.expect("screenshot failed");
        let calls = driver.get_calls();
        assert_eq!(calls.len(), 1, "exactly one call SHALL be recorded");
        assert_eq!(
            calls[0].0, "screenshot",
            "the call SHALL be named screenshot"
        );
        assert!(
            calls[0].1.is_empty(),
            "screenshot SHALL record no arguments"
        );
    }

    // 7. push_notification with no payload records only title + body.
    #[tokio::test]
    async fn mock_push_notification_without_payload() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver
            .push_notification("T", "B", None)
            .await
            .expect("push failed");
        let calls = driver.get_calls();
        assert_eq!(calls[0].0, "push_notification");
        assert_eq!(
            calls[0].1,
            vec!["T", "B"],
            "absent payload SHALL omit the third arg"
        );
    }

    // 8. push_notification with a payload appends it as a third arg.
    #[tokio::test]
    async fn mock_push_notification_with_payload() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver
            .push_notification("T", "B", Some("{\"k\":1}"))
            .await
            .expect("push failed");
        let calls = driver.get_calls();
        assert_eq!(
            calls[0].1,
            vec!["T", "B", "{\"k\":1}"],
            "present payload SHALL be appended as the third arg"
        );
    }

    // 9. stop_recording records exactly one no-arg "stop_recording" call.
    #[tokio::test]
    async fn mock_stop_recording_records_noarg_call() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver
            .stop_recording()
            .await
            .expect("stop_recording failed");
        let calls = driver.get_calls();
        assert_eq!(calls.len(), 1, "exactly one call SHALL be recorded");
        assert_eq!(
            calls[0].0, "stop_recording",
            "the call SHALL be named stop_recording"
        );
        assert!(
            calls[0].1.is_empty(),
            "stop_recording SHALL record no arguments"
        );
    }

    // 10. The remaining single/double-string-arg methods all record their
    //     name and arguments verbatim.
    #[tokio::test]
    async fn mock_records_remaining_methods_verbatim() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let launch_warning = driver.launch_app("com.x").await.expect("launch failed");
        assert!(
            launch_warning.is_none(),
            "mock launch_app SHALL surface no warning"
        );
        driver.stop_app("com.x").await.expect("stop failed");
        driver.clear_app_data("com.x").await.expect("clear failed");
        driver.press_button("home").await.expect("press failed");
        driver.set_dark_mode(true).await.expect("dark failed");
        driver.set_location(1.5, -2.5).await.expect("loc failed");
        driver.open_url("https://x").await.expect("url failed");
        driver.add_media("/tmp/a.png").await.expect("media failed");
        driver
            .grant_permission("com.x", "camera")
            .await
            .expect("grant failed");
        driver
            .revoke_permission("com.x", "camera")
            .await
            .expect("revoke failed");
        driver.start_recording("rec").await.expect("rec failed");
        driver.remove_port_forwards().await.expect("ports failed");

        let calls = driver.get_calls();
        let expected: Vec<(&str, Vec<&str>)> = vec![
            ("launch_app", vec!["com.x"]),
            ("stop_app", vec!["com.x"]),
            ("clear_app_data", vec!["com.x"]),
            ("press_button", vec!["home"]),
            ("set_dark_mode", vec!["true"]),
            ("set_location", vec!["1.5", "-2.5"]),
            ("open_url", vec!["https://x"]),
            ("add_media", vec!["/tmp/a.png"]),
            ("grant_permission", vec!["com.x", "camera"]),
            ("revoke_permission", vec!["com.x", "camera"]),
            ("start_recording", vec!["rec"]),
            ("remove_port_forwards", vec![]),
        ];
        assert_eq!(calls.len(), expected.len());
        for (i, (name, args)) in expected.iter().enumerate() {
            assert_eq!(&calls[i].0, name, "call {i} name SHALL match");
            assert_eq!(&calls[i].1, args, "call {i} args SHALL match");
        }
    }

    // ── MockPlatformDriver: keyboard height + call bookkeeping ─────

    // 12. set_keyboard_height is reflected in the next get_hierarchy meta.
    #[tokio::test]
    async fn mock_get_hierarchy_reports_keyboard_height() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.set_keyboard_height(291);
        let (_, meta) = driver.get_hierarchy().await.expect("get_hierarchy failed");
        assert_eq!(
            meta.keyboard_height, 291,
            "meta SHALL reflect the configured keyboard height"
        );
    }

    // 13. hide_keyboard records the call and zeroes the keyboard height so
    //     the next get_hierarchy reports 0.
    #[tokio::test]
    async fn mock_hide_keyboard_zeroes_height() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.set_keyboard_height(300);
        driver.hide_keyboard().await.expect("hide_keyboard failed");
        let (_, meta) = driver.get_hierarchy().await.expect("get_hierarchy failed");
        assert_eq!(
            meta.keyboard_height, 0,
            "hide_keyboard SHALL zero the keyboard height"
        );
        // The call log holds hide_keyboard then get_hierarchy.
        let calls = driver.get_calls();
        assert_eq!(calls[0].0, "hide_keyboard");
    }

    // 14. clear_calls empties the recorded-call log without affecting
    //     subsequent recording.
    #[tokio::test]
    async fn mock_clear_calls_resets_log() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.tap(1, 1).await.expect("tap failed");
        assert_eq!(driver.get_calls().len(), 1);
        driver.clear_calls();
        assert!(
            driver.get_calls().is_empty(),
            "clear_calls SHALL empty the log"
        );
        driver.tap(2, 2).await.expect("tap failed");
        assert_eq!(
            driver.get_calls().len(),
            1,
            "recording SHALL resume after clear_calls"
        );
    }

    // ── default trait methods (no-op implementations) ──────────────

    // 15. poke_for_system_alert's default impl is a no-op that returns Ok
    //     and records nothing.
    #[tokio::test]
    async fn mock_poke_for_system_alert_is_noop_ok() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver
            .poke_for_system_alert()
            .await
            .expect("poke SHALL be a no-op Ok");
        assert!(
            driver.get_calls().is_empty(),
            "default poke SHALL record nothing"
        );
    }

    // 16. set_request_timeout's default impl is a no-op that records
    //     nothing and does not panic.
    #[test]
    fn mock_set_request_timeout_is_noop() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        driver.set_request_timeout(std::time::Duration::from_secs(3));
        assert!(
            driver.get_calls().is_empty(),
            "default set_request_timeout SHALL record nothing"
        );
    }

    // ── plain value-type derives ───────────────────────────────────
}
