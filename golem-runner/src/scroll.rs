use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::{Direction, PlatformDriver};
use golem_element::selector::{find_elements, Selector};
use golem_element::{filter_viewport, Element, FindResult, Viewport};
use tokio::time::Instant;

use crate::resolution::wait_for_settle;

/// Default maximum number of scroll attempts before giving up.
pub const DEFAULT_MAX_SCROLLS: u32 = 20;

/// Serialize an element hierarchy to a string for comparison.
///
/// This is used for bounce detection: when a swipe produces the same
/// hierarchy as before, we know we've hit the end of the scrollable area.
fn hierarchy_fingerprint(root: &Element) -> String {
    let mut buf = String::new();
    build_fingerprint(root, &mut buf);
    buf
}

fn build_fingerprint(element: &Element, buf: &mut String) {
    buf.push_str(&element.element_type);
    buf.push(':');
    if let Some(ref text) = element.text {
        buf.push_str(text);
    }
    buf.push(':');
    if let Some(ref id) = element.accessibility_label {
        buf.push_str(id);
    }
    // Include bounds so scroll position changes are detected (WebViews report
    // absolute document coordinates that shift as the page scrolls).
    let b = &element.bounds;
    buf.push_str(&format!("@{},{}", b.x, b.y));
    buf.push('[');
    for child in &element.children {
        build_fingerprint(child, buf);
        buf.push(',');
    }
    buf.push(']');
}

/// Return the opposite scroll direction.
fn reverse_direction(dir: Direction) -> Direction {
    match dir {
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
    }
}

/// Compute swipe coordinates: the swipe starts at `(start_x, start_y)` and
/// travels `swipe_pct`% of the screen in the given direction.
///
/// Clamps all coordinates to 10%-90% of the screen to avoid system gesture
/// areas (notification bar, home indicator). Shorter swipes near edges are
/// fine — a short scroll beats a wasted one.
fn swipe_from(
    viewport: &Viewport,
    direction: Direction,
    start_x: i32,
    start_y: i32,
    swipe_pct: u32,
) -> (i32, i32, i32, i32) {
    let dy = viewport.height * swipe_pct as i32 / 100;
    let dx = viewport.width * swipe_pct as i32 / 100;

    let min_x = viewport.width / 10;
    let max_x = viewport.width * 9 / 10;
    let min_y = viewport.height / 10;
    let max_y = viewport.height * 9 / 10;

    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi);

    // Direction is the scroll intent (where the user wants content to go),
    // not the finger direction. "Down" = see content below = finger swipes up.
    let (fx, fy, tx, ty) = match direction {
        Direction::Down => (start_x, start_y, start_x, start_y - dy),
        Direction::Up => (start_x, start_y, start_x, start_y + dy),
        Direction::Left => (start_x, start_y, start_x + dx, start_y),
        Direction::Right => (start_x, start_y, start_x - dx, start_y),
    };

    (
        clamp(fx, min_x, max_x),
        clamp(fy, min_y, max_y),
        clamp(tx, min_x, max_x),
        clamp(ty, min_y, max_y),
    )
}

/// Default swipe start position: finger starts near the trailing edge
/// (opposite to scroll intent) so it has room to travel AND avoids inner
/// scrollable elements that typically occupy the middle of the screen.
/// "Down" scroll = finger starts at 80% from top (below most inner scrollables).
fn default_swipe_start(viewport: &Viewport, direction: Direction) -> (i32, i32) {
    let cx = viewport.width / 2;
    let cy = viewport.height / 2;
    match direction {
        Direction::Down => (cx, viewport.height * 80 / 100),
        Direction::Up => (cx, viewport.height * 20 / 100),
        Direction::Left => (viewport.width * 20 / 100, cy),
        Direction::Right => (viewport.width * 80 / 100, cy),
    }
}

/// Alternate swipe start positions to try when the default position is
/// consumed by an inner scrollable element.
///
/// The start position determines which scrollable captures the gesture.
/// Probes try starting above/below (for vertical) and left/right (for
/// full-height inner scrollables) of the inner element.
/// Probe start positions: the start position determines which scrollable
/// captures the gesture. For "Down" scroll (finger starts low, swipes up),
/// probes try starting at different Y positions to avoid inner scrollables,
/// plus left/right X for full-height inner scrollables.
fn probe_starts(viewport: &Viewport, direction: Direction) -> Vec<(i32, i32)> {
    let cx = viewport.width / 2;
    let cy = viewport.height / 2;
    match direction {
        Direction::Down => vec![
            // Finger starts low and swipes up. Try positions outside common
            // inner scrollable ranges (typically 30%-80% of screen).
            (cx, viewport.height * 15 / 100),     // near top — above inner scrollables
            (cx, viewport.height * 90 / 100),     // near bottom — below inner scrollables
            (viewport.width * 85 / 100, viewport.height * 80 / 100), // right edge, low
            (viewport.width * 15 / 100, viewport.height * 80 / 100), // left edge, low
        ],
        Direction::Up => vec![
            (cx, viewport.height * 85 / 100),     // near bottom — below inner scrollables
            (cx, viewport.height * 10 / 100),     // near top — above inner scrollables
            (viewport.width * 85 / 100, viewport.height * 20 / 100), // right edge, high
            (viewport.width * 15 / 100, viewport.height * 20 / 100), // left edge, high
        ],
        Direction::Left => vec![
            (viewport.width * 85 / 100, cy),      // near right — beyond inner scrollables
            (viewport.width * 10 / 100, cy),      // near left
            (viewport.width * 20 / 100, viewport.height * 85 / 100), // low left
            (viewport.width * 20 / 100, viewport.height * 15 / 100), // high left
        ],
        Direction::Right => vec![
            (viewport.width * 15 / 100, cy),      // near left
            (viewport.width * 90 / 100, cy),      // near right
            (viewport.width * 80 / 100, viewport.height * 85 / 100), // low right
            (viewport.width * 80 / 100, viewport.height * 15 / 100), // high right
        ],
    }
}

/// Choose swipe distance based on how far away the target element is.
/// - Close (< 1 screen): 40% swipe
/// - Medium (1-2 screens): 60% swipe
/// - Far (> 2 screens): 80% swipe
fn swipe_pct_for_distance(distance_ratio: f32) -> u32 {
    if distance_ratio > 2.0 { return 80; }
    if distance_ratio > 1.0 { return 60; }
    40
}

/// Scroll through a view to find an element matching the given selector.
///
/// The algorithm:
/// 1. Check if the element already exists in the current hierarchy.
/// 2. Swipe in the given direction at the current anchor point.
/// 3. Get the new hierarchy and check again.
/// 4. If the swipe was wasted (hierarchy unchanged), probe alternate positions
///    (right, left, lower, upper edges — one attempt each) to avoid inner
///    scrollable elements consuming the gesture.
/// 5. If a probe works, adopt that anchor for subsequent swipes.
/// 6. If the hierarchy still hasn't changed after probing, treat as bounce
///    and reverse direction.
/// 7. If both directions are exhausted, return an error.
/// 8. If max_scrolls is reached, return an error.
pub async fn scroll_to_element(
    selector: &Selector,
    driver: &dyn PlatformDriver,
    initial_direction: Direction,
    max_scrolls: u32,
) -> Result<FindResult> {
    scroll_to_element_with_hint(selector, driver, initial_direction, max_scrolls, 0.0, None, None).await
}

/// Like `scroll_to_element`, but accepts hints from the caller:
/// - `distance_ratio`: how many screens away the target is (for adaptive swipe speed)
/// - `timeout_ms`: optional time limit for the scroll operation
/// - `container`: optional bounds to constrain swipes within (for `within` selector)
pub async fn scroll_to_element_with_hint(
    selector: &Selector,
    driver: &dyn PlatformDriver,
    initial_direction: Direction,
    max_scrolls: u32,
    distance_ratio: f32,
    timeout_ms: Option<u64>,
    container: Option<golem_element::Bounds>,
) -> Result<FindResult> {
    // Step 1: Check current viewport-filtered hierarchy before any scrolling.
    let mut root = wait_for_settle(driver).await?;
    let viewport = Viewport::from_root(&root);
    let visible = filter_viewport(&root, &viewport);
    let results = find_elements(&visible, selector);
    if let Some(found) = results.into_iter().next() {
        return Ok(found);
    }

    let mut direction = initial_direction;
    let mut reversed = false;
    let mut prev_fingerprint = hierarchy_fingerprint(&root);
    let pct = swipe_pct_for_distance(distance_ratio);
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));

    let mut start = if container.is_some() {
        // Start inside the container, clamped to the visible portion of the screen.
        let cb = container.as_ref().unwrap();
        // Visible portion of container: clamp to viewport
        let vis_top = cb.y.max(0);
        let vis_bot = (cb.y + cb.height).min(viewport.height);
        let vis_left = cb.x.max(0);
        let vis_right = (cb.x + cb.width).min(viewport.width);
        let vis_cy = (vis_top + vis_bot) / 2;
        let vis_cx = (vis_left + vis_right) / 2;
        match direction {
            Direction::Down => (vis_cx, vis_top + (vis_bot - vis_top) * 70 / 100),
            Direction::Up => (vis_cx, vis_top + (vis_bot - vis_top) * 30 / 100),
            Direction::Left => (vis_left + (vis_right - vis_left) * 30 / 100, vis_cy),
            Direction::Right => (vis_left + (vis_right - vis_left) * 70 / 100, vis_cy),
        }
    } else {
        default_swipe_start(&viewport, direction)
    };

    for _ in 0..max_scrolls {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            bail!(
                "Scroll timed out after {}ms: text={:?}, id={:?}",
                timeout_ms.unwrap_or(0),
                selector.text,
                selector.accessibility_label,
            );
        }
        let (fx, fy, tx, ty) = if let Some(ref cb) = container {
            // Swipe within the visible portion of the container.
            let vis_top = cb.y.max(0);
            let vis_bot = (cb.y + cb.height).min(viewport.height);
            let vis_left = cb.x.max(0);
            let vis_right = (cb.x + cb.width).min(viewport.width);
            let vis_h = vis_bot - vis_top;
            let vis_w = vis_right - vis_left;
            let dy = vis_h * 80 / 100;
            let dx = vis_w * 80 / 100;
            let clamp_x = |v: i32| v.max(vis_left + 5).min(vis_right - 5);
            let clamp_y = |v: i32| v.max(vis_top + 5).min(vis_bot - 5);
            let (fx, fy, tx, ty) = match direction {
                Direction::Down => (start.0, start.1, start.0, start.1 - dy),
                Direction::Up => (start.0, start.1, start.0, start.1 + dy),
                Direction::Left => (start.0, start.1, start.0 + dx, start.1),
                Direction::Right => (start.0, start.1, start.0 - dx, start.1),
            };
            (clamp_x(fx), clamp_y(fy), clamp_x(tx), clamp_y(ty))
        } else {
            swipe_from(&viewport, direction, start.0, start.1, pct)
        };
        driver.swipe_coords(fx, fy, tx, ty).await?;

        root = wait_for_settle(driver).await?;
        let vp = Viewport::from_root(&root);
        let visible = filter_viewport(&root, &vp);
        let results = find_elements(&visible, selector);
        if let Some(found) = results.into_iter().next() {
            return Ok(found);
        }

        let new_fingerprint = hierarchy_fingerprint(&root);
        if new_fingerprint == prev_fingerprint {
            // When scrolling within a container, skip edge probing — the
            // container IS the target scrollable. Just detect boundary bounce.
            if container.is_none() {
                // Wasted swipe — an inner scrollable may have consumed it.
                // Probe alternate start positions (one attempt each).
                let mut probed_ok = false;
                for (px, py) in probe_starts(&viewport, direction) {
                    let (fx, fy, tx, ty) = swipe_from(&viewport, direction, px, py, 40);
                    driver.swipe_coords(fx, fy, tx, ty).await?;
                    root = wait_for_settle(driver).await?;

                    let probe_fp = hierarchy_fingerprint(&root);
                    if probe_fp != prev_fingerprint {
                        start = (px, py);
                        prev_fingerprint = probe_fp;
                        probed_ok = true;

                        let vp = Viewport::from_root(&root);
                        let visible = filter_viewport(&root, &vp);
                        let results = find_elements(&visible, selector);
                        if let Some(found) = results.into_iter().next() {
                            return Ok(found);
                        }
                        break;
                    }
                }

                if probed_ok {
                    continue;
                }
            }

            // True boundary reached (or container scroll exhausted).
            if reversed {
                bail!(
                    "Element not found: scrolled in both directions and hit boundaries. \
                     Selector: text={:?}, id={:?}",
                    selector.text,
                    selector.accessibility_label,
                );
            }
            direction = reverse_direction(direction);
            reversed = true;
            if container.is_some() {
                let cb = container.as_ref().unwrap();
                let vis_top = cb.y.max(0);
                let vis_bot = (cb.y + cb.height).min(viewport.height);
                let vis_left = cb.x.max(0);
                let vis_right = (cb.x + cb.width).min(viewport.width);
                let vis_cx = (vis_left + vis_right) / 2;
                start = match direction {
                    Direction::Down => (vis_cx, vis_top + (vis_bot - vis_top) * 70 / 100),
                    Direction::Up => (vis_cx, vis_top + (vis_bot - vis_top) * 30 / 100),
                    Direction::Left => (vis_left + (vis_right - vis_left) * 30 / 100, (vis_top + vis_bot) / 2),
                    Direction::Right => (vis_left + (vis_right - vis_left) * 70 / 100, (vis_top + vis_bot) / 2),
                };
            } else {
                start = default_swipe_start(&viewport, direction);
            }
        } else {
            prev_fingerprint = new_fingerprint;
        }
    }

    // Step 7: Max scrolls reached
    bail!(
        "Element not found after {max_scrolls} scroll attempts. \
         Selector: text={:?}, id={:?}",
        selector.text,
        selector.accessibility_label,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    // ── Test helpers ──────────────────────────────────────────────────

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
            children: Vec::new(),
        }
    }

    fn make_element_with_text(element_type: &str, text: &str, bounds: Bounds) -> Element {
        let mut e = make_element(element_type, bounds);
        e.text = Some(text.to_string());
        e
    }

    fn default_bounds() -> Bounds {
        Bounds::new(0, 0, 375, 812)
    }

    /// Determine scroll intent from recorded swipe_coords call args.
    /// The scroll intent is opposite to the finger movement: finger swipes up
    /// means content scrolls down (user sees content below).
    fn scroll_intent(args: &[String]) -> &'static str {
        let from_y: i32 = args[1].parse().unwrap();
        let to_y: i32 = args[3].parse().unwrap();
        let from_x: i32 = args[0].parse().unwrap();
        let to_x: i32 = args[2].parse().unwrap();
        let dy = to_y - from_y;
        let dx = to_x - from_x;
        // Invert: finger up = scroll down, finger left = scroll right
        if dy.abs() > dx.abs() {
            if dy < 0 { "Down" } else { "Up" }
        } else {
            if dx < 0 { "Right" } else { "Left" }
        }
    }

    fn sel_with_text(text: &str) -> Selector {
        Selector {
            text: Some(text.to_string()),
            ..Selector::default()
        }
    }

    /// A mock driver that returns different hierarchies on successive
    /// `get_hierarchy()` calls, allowing us to simulate scrolling.
    struct SequenceMockDriver {
        hierarchies: Mutex<Vec<Element>>,
        call_index: AtomicU32,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl SequenceMockDriver {
        /// Create a mock that returns each hierarchy twice (for settle compatibility).
        /// wait_for_settle needs two consecutive identical snapshots to consider
        /// the UI stable, so each logical step requires a duplicate entry.
        fn new(hierarchies: Vec<Element>) -> Self {
            let doubled: Vec<Element> = hierarchies
                .into_iter()
                .flat_map(|h| [h.clone(), h])
                .collect();
            Self {
                hierarchies: Mutex::new(doubled),
                call_index: AtomicU32::new(0),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn get_calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().expect("lock poisoned").clone()
        }

        fn record_call(&self, method: &str, args: Vec<String>) {
            self.calls
                .lock()
                .expect("lock poisoned")
                .push((method.to_string(), args));
        }
    }

    #[async_trait::async_trait]
    impl PlatformDriver for SequenceMockDriver {
        async fn get_hierarchy(&self) -> anyhow::Result<Element> {
            self.record_call("get_hierarchy", vec![]);
            let hierarchies = self.hierarchies.lock().expect("lock poisoned");
            let idx = self.call_index.fetch_add(1, Ordering::SeqCst) as usize;
            // Clamp to last hierarchy if we exceed the sequence
            let clamped = idx.min(hierarchies.len().saturating_sub(1));
            Ok(hierarchies[clamped].clone())
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

        async fn swipe(&self, direction: Direction) -> anyhow::Result<()> {
            self.record_call("swipe", vec![format!("{direction:?}")]);
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

        async fn screenshot(&self) -> anyhow::Result<golem_driver::ScreenshotResult> {
            self.record_call("screenshot", vec![]);
            Ok(golem_driver::ScreenshotResult {
                path: "mock.png".to_string(),
                data: vec![],
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

        async fn grant_permission(
            &self,
            bundle_id: &str,
            permission: &str,
        ) -> anyhow::Result<()> {
            self.record_call(
                "grant_permission",
                vec![bundle_id.to_string(), permission.to_string()],
            );
            Ok(())
        }

        async fn revoke_permission(
            &self,
            bundle_id: &str,
            permission: &str,
        ) -> anyhow::Result<()> {
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
            Ok("mock.mp4".to_string())
        }

        async fn get_alert(&self) -> anyhow::Result<Option<Element>> {
            self.record_call("get_alert", vec![]);
            Ok(None)
        }

        async fn dismiss_alert(&self, button: Option<&str>) -> anyhow::Result<()> {
            self.record_call(
                "dismiss_alert",
                button
                    .map(|b| vec![b.to_string()])
                    .unwrap_or_default(),
            );
            Ok(())
        }

        async fn remove_port_forwards(&self) -> anyhow::Result<()> {
            self.record_call("remove_port_forwards", vec![]);
            Ok(())
        }
    }

    // ── 1. Element found in initial hierarchy (no scroll needed) ─────

    #[tokio::test]
    async fn element_found_in_initial_hierarchy() {
        let mut root = make_element("View", default_bounds());
        root.children.push(make_element_with_text(
            "Button",
            "Target",
            Bounds::new(10, 10, 100, 44),
        ));

        let driver = MockPlatformDriver::new(root);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20)
            .await
            .expect("should find element without scrolling");

        assert_eq!(result.element.text.as_deref(), Some("Target"));
        // No swipe calls should have been made
        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert!(swipe_calls.is_empty(), "no swipes SHALL occur");
    }

    // ── 2. Element found after one scroll ───────────────────────────

    #[tokio::test]
    async fn element_found_after_one_scroll() {
        // First hierarchy: no target
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page 1",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        // Second hierarchy (after scroll): target appears
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Target",
                Bounds::new(10, 100, 100, 44),
            ));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20)
            .await
            .expect("should find element after one scroll");

        assert_eq!(result.element.text.as_deref(), Some("Target"));

        // Should have exactly one swipe call
        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Down");
    }

    // ── 3. Element not found after max_scrolls → error ──────────────

    #[tokio::test]
    async fn element_not_found_after_max_scrolls() {
        // Create hierarchies that all change but never contain the target.
        let hierarchies: Vec<Element> = (0..25)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text(
                    "Label",
                    &format!("Page {i}"),
                    Bounds::new(0, 0, 200, 40),
                ));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Nonexistent");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 5).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("not found after 5 scroll attempts"),
            "error should mention max scrolls, got: {err_msg}"
        );
    }

    // ── 4. Bounce detection: hierarchy unchanged triggers direction reversal ─

    #[tokio::test]
    async fn bounce_detection_triggers_direction_reversal() {
        // When a center swipe is wasted (same hierarchy), edge probes are tried
        // first (4 attempts). If all probes also waste, direction reverses.
        let base = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Static Page",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let different = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Different Page",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };

        // Need enough identical entries for: initial settle(2) + center swipe settle(2)
        // + 4 probes × settle(2) + reversed swipe settle(2) + more
        let mut seq: Vec<Element> = std::iter::repeat(base.clone()).take(14).collect();
        // After probes exhaust and direction reverses, return different hierarchy
        seq.push(different.clone());
        seq.push(different.clone());
        seq.push(different.clone());
        seq.push(different.clone());

        let driver = SequenceMockDriver::new(seq);
        let selector = sel_with_text("Nonexistent");

        let _ = scroll_to_element(&selector, &driver, Direction::Down, 10).await;

        // Check that swipe direction changed: first batch is Down (center + probes),
        // then Up after reversal.
        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        let directions: Vec<&str> = swipe_calls.iter().map(|c| scroll_intent(&c.1)).collect();
        // Should have Down swipes (center + 4 probes), then at least one Up
        assert!(
            directions.contains(&"Up"),
            "direction should reverse after probes exhaust, got: {directions:?}"
        );
        let first_up = directions.iter().position(|&d| d == "Up").unwrap();
        assert!(
            directions[..first_up].iter().all(|&d| d == "Down"),
            "all swipes before reversal should be Down"
        );
    }

    // ── 5. Element found after direction reversal ───────────────────

    #[tokio::test]
    async fn element_found_after_direction_reversal() {
        let base = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Bottom Page",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let with_target = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Target",
                Bounds::new(10, 100, 100, 44),
            ));
            root
        };

        // Doubled entries: initial settle(2) + center swipe settle(2) [bounce]
        // + 4 probes × settle(2) = 12 entries of base. Then reversed swipe
        // settle(2) returns with_target → found.
        // 6 base (doubled to 12) + with_target at positions 12+.
        let mut seq: Vec<Element> = std::iter::repeat(base.clone()).take(6).collect();
        seq.push(with_target.clone());
        seq.push(with_target.clone());

        let driver = SequenceMockDriver::new(seq);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20)
            .await
            .expect("should find element after direction reversal");

        assert_eq!(result.element.text.as_deref(), Some("Target"));

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        let directions: Vec<&str> = swipe_calls.iter().map(|c| scroll_intent(&c.1)).collect();
        // Should have Down swipes (center + probes), then Up (reversed) which finds target
        assert!(
            directions.contains(&"Up"),
            "should reverse and find target, got: {directions:?}"
        );
    }

    // ── 6. Max scrolls reached returns appropriate error ────────────

    #[tokio::test]
    async fn max_scrolls_reached_returns_error() {
        // All different hierarchies but none contain the target
        let hierarchies: Vec<Element> = (0..10)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text(
                    "Label",
                    &format!("Screen {i}"),
                    Bounds::new(0, 0, 200, 40),
                ));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Missing");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 3).await;
        assert!(result.is_err());
        let err = result.expect_err("should error");
        let msg = format!("{err}");
        assert!(
            msg.contains("not found after 3 scroll attempts"),
            "error should cite max scrolls: {msg}"
        );
    }

    // ── 7. Empty hierarchy returns error ────────────────────────────

    #[tokio::test]
    async fn empty_hierarchy_returns_error() {
        // Root with no children
        let root = make_element("View", default_bounds());
        let driver = MockPlatformDriver::new(root);
        let selector = sel_with_text("Anything");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 3).await;
        assert!(result.is_err());
    }

    // ── 8. Scroll down direction correct ────────────────────────────

    #[tokio::test]
    async fn scroll_down_direction_correct() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10, 10, 100, 44),
            ));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Found");

        scroll_to_element(&selector, &driver, Direction::Down, 20)
            .await
            .expect("should find element");

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Down");
    }

    // ── 9. Scroll up direction correct ──────────────────────────────

    #[tokio::test]
    async fn scroll_up_direction_correct() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10, 10, 100, 44),
            ));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Found");

        scroll_to_element(&selector, &driver, Direction::Up, 20)
            .await
            .expect("should find element");

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Up");
    }

    // ── 10. Scroll left direction works ─────────────────────────────

    #[tokio::test]
    async fn scroll_left_direction_works() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10, 10, 100, 44),
            ));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Found");

        scroll_to_element(&selector, &driver, Direction::Left, 20)
            .await
            .expect("should find element");

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Left");
    }

    // ── 11. Scroll right direction works ────────────────────────────

    #[tokio::test]
    async fn scroll_right_direction_works() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10, 10, 100, 44),
            ));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Found");

        scroll_to_element(&selector, &driver, Direction::Right, 20)
            .await
            .expect("should find element");

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Right");
    }

    // ── 12. Default max_scrolls behavior ────────────────────────────

    #[tokio::test]
    async fn default_max_scrolls_behavior() {
        assert_eq!(DEFAULT_MAX_SCROLLS, 20);

        // Create enough distinct hierarchies to exhaust 20 scrolls
        let hierarchies: Vec<Element> = (0..25)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text(
                    "Label",
                    &format!("Screen {i}"),
                    Bounds::new(0, 0, 200, 40),
                ));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Nonexistent");

        let result =
            scroll_to_element(&selector, &driver, Direction::Down, DEFAULT_MAX_SCROLLS).await;
        assert!(result.is_err());

        // Should have made exactly DEFAULT_MAX_SCROLLS swipe calls
        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), DEFAULT_MAX_SCROLLS as usize);
    }

    // ── 13. Element found on last allowed scroll ────────────────────

    #[tokio::test]
    async fn element_found_on_last_allowed_scroll() {
        let max_scrolls = 3_u32;
        // Build a sequence: initial (no target), scroll 1 (no), scroll 2 (no), scroll 3 (found!)
        // get_hierarchy calls: initial + 3 after swipes = 4 calls
        let mut hierarchies: Vec<Element> = (0..3)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text(
                    "Label",
                    &format!("Page {i}"),
                    Bounds::new(0, 0, 200, 40),
                ));
                root
            })
            .collect();

        // 4th hierarchy (after 3rd swipe): target found
        let mut target_root = make_element("View", default_bounds());
        target_root.children.push(make_element_with_text(
            "Button",
            "Target",
            Bounds::new(10, 10, 100, 44),
        ));
        hierarchies.push(target_root);

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, max_scrolls)
            .await
            .expect("should find element on last scroll");

        assert_eq!(result.element.text.as_deref(), Some("Target"));

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe_coords")
            .collect();
        assert_eq!(swipe_calls.len(), max_scrolls as usize);
    }

    // ── 14. Double bounce (both directions exhausted) → error ───────

    #[tokio::test]
    async fn double_bounce_both_directions_exhausted() {
        let static_page = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Static Content",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };
        let different_page = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Different Content",
                Bounds::new(0, 0, 200, 40),
            ));
            root
        };

        // Sequence:
        // Call 0 (initial): static_page (no target)
        // Call 1 (after swipe Down): static_page again => BOUNCE, reverse to Up
        // Call 2 (after swipe Up): different_page (no target, no bounce)
        // Call 3 (after swipe Up): different_page again => BOUNCE, already reversed => error
        let driver = SequenceMockDriver::new(vec![
            static_page.clone(),
            static_page,         // bounce #1
            different_page.clone(),
            different_page,      // bounce #2
        ]);
        let selector = sel_with_text("Nonexistent");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("both directions"),
            "error should mention both directions exhausted, got: {err_msg}"
        );
    }
}
