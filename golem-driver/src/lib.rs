pub mod android;
pub mod cdp;
pub mod common;
pub mod commands;
pub mod ios;

use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG: AtomicBool = AtomicBool::new(false);

/// Enable debug logging for driver-level diagnostics (WebKit/CDP).
pub fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

pub fn is_debug() -> bool {
    DEBUG.load(Ordering::Relaxed)
}
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
    /// Returns `Ok(())` on settle OR on deadline. Doesn't fail launch
    /// — if the gate is wrong, the downstream action's own timeout
    /// catches genuinely broken cases.
    async fn await_first_frame(&self) -> anyhow::Result<()> {
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

/// Hard deadline. Beyond this we proceed anyway — the downstream action's
/// own timeout will catch genuinely broken UI states.
pub const AWAIT_FIRST_FRAME_DEADLINE: std::time::Duration =
    std::time::Duration::from_secs(10);

/// Number of consecutive stable polls required to declare settle.
const STABLE_POLLS_REQUIRED: u32 = 2;

/// Default settle implementation — shared between iOS and Android since
/// both use the same `get_hierarchy` companion endpoint. Drivers can
/// override `await_first_frame` to add platform-specific signals (e.g.
/// WebKit Inspector readiness on iOS WebView screens) but for native
/// flows this is sufficient.
async fn await_first_frame_default(
    driver: &(impl PlatformDriver + ?Sized),
) -> anyhow::Result<()> {
    // Use `tokio::time::Instant` (not `std::time`) so this respects
    // tokio's paused-time test mode. Otherwise unit tests that simulate
    // the 10s deadline would burn real wall-clock.
    let start = tokio::time::Instant::now();
    let deadline = start + AWAIT_FIRST_FRAME_DEADLINE;
    let mut prev_count: usize = 0;
    let mut stable_polls: u32 = 0;
    loop {
        if tokio::time::Instant::now() >= deadline {
            if is_debug() {
                eprintln!(
                    "  [launch] settle deadline reached after {:?}, last seen {prev_count} nodes — proceeding anyway",
                    start.elapsed()
                );
            }
            return Ok(());
        }
        let count = match driver.get_hierarchy().await {
            Ok((tree, _)) => tree.node_count(),
            // Tree-fetch errors mid-launch are common (companion port
            // briefly unresponsive). Treat as 0 nodes and keep polling
            // — bubbling the error would fail launch entirely.
            Err(_) => 0,
        };
        if count >= AWAIT_FIRST_FRAME_MIN_NODES && count == prev_count {
            stable_polls += 1;
            if stable_polls >= STABLE_POLLS_REQUIRED {
                if is_debug() {
                    eprintln!(
                        "  [launch] UI settled in {:?} ({count} nodes)",
                        start.elapsed()
                    );
                }
                return Ok(());
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
}

impl MockPlatformDriver {
    pub fn new(hierarchy: Element) -> Self {
        Self {
            hierarchy: Mutex::new(hierarchy),
            calls: Mutex::new(Vec::new()),
            keyboard_height: Mutex::new(0),
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
}

#[async_trait]
impl PlatformDriver for MockPlatformDriver {
    async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
        self.record_call("get_hierarchy", vec![]);
        let meta = common::HierarchyMeta {
            keyboard_height: *self.keyboard_height.lock().expect("lock poisoned"),
            ..common::HierarchyMeta::default()
        };
        Ok((self.hierarchy.lock().expect("lock poisoned").clone(), meta))
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
        // Mirror real driver behaviour: dismissing the keyboard zeroes
        // the height the next get_hierarchy() reports.
        *self.keyboard_height.lock().expect("lock poisoned") = 0;
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
            let mut frames: Vec<Element> = progression
                .into_iter()
                .map(tree_with_children)
                .collect();
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
        async fn tap(&self, _x: i32, _y: i32) -> anyhow::Result<()> { unimplemented!() }
        async fn long_press(&self, _x: i32, _y: i32, _d: u64) -> anyhow::Result<()> { unimplemented!() }
        async fn type_text(&self, _t: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn backspace(&self, _c: u32) -> anyhow::Result<()> { unimplemented!() }
        async fn swipe_coords(&self, _: i32, _: i32, _: i32, _: i32) -> anyhow::Result<()> { unimplemented!() }
        async fn pinch(&self, _x: i32, _y: i32, _s: f64, _v: f64) -> anyhow::Result<()> { unimplemented!() }
        async fn gesture(&self, _f: Vec<GestureFinger>) -> anyhow::Result<()> { unimplemented!() }
        async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> { unimplemented!() }
        async fn hide_keyboard(&self) -> anyhow::Result<()> { unimplemented!() }
        async fn launch_app(&self, _b: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn stop_app(&self, _b: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn clear_app_data(&self, _b: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn press_button(&self, _b: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn set_dark_mode(&self, _e: bool) -> anyhow::Result<()> { unimplemented!() }
        async fn set_location(&self, _: f64, _: f64) -> anyhow::Result<()> { unimplemented!() }
        async fn open_url(&self, _u: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn push_notification(&self, _: &str, _: &str, _: Option<&str>) -> anyhow::Result<()> { unimplemented!() }
        async fn add_media(&self, _p: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn grant_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn revoke_permission(&self, _b: &str, _p: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn start_recording(&self, _n: &str) -> anyhow::Result<()> { unimplemented!() }
        async fn stop_recording(&self) -> anyhow::Result<String> { unimplemented!() }
        async fn remove_port_forwards(&self) -> anyhow::Result<()> { unimplemented!() }
    }

    // Helper picks a child count safely above the MIN_NODES threshold so
    // the gate accepts the resulting tree as a real first screen.
    fn above_threshold() -> usize {
        AWAIT_FIRST_FRAME_MIN_NODES + 10
    }

    fn below_threshold() -> usize {
        AWAIT_FIRST_FRAME_MIN_NODES.saturating_sub(5)
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
}
