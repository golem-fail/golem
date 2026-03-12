use anyhow::{bail, Result};
use golem_driver::{Direction, PlatformDriver};
use golem_element::selector::{find_elements, Selector};
use golem_element::{Element, FindResult};

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
    if let Some(ref id) = element.id {
        buf.push_str(id);
    }
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

/// Scroll through a view to find an element matching the given selector.
///
/// The algorithm:
/// 1. Check if the element already exists in the current hierarchy.
/// 2. Swipe in the given direction.
/// 3. Get the new hierarchy and check again.
/// 4. If the hierarchy hasn't changed (bounce detection), reverse direction.
/// 5. If both directions are exhausted, return an error.
/// 6. If max_scrolls is reached, return an error.
pub async fn scroll_to_element(
    selector: &Selector,
    driver: &dyn PlatformDriver,
    initial_direction: Direction,
    max_scrolls: u32,
) -> Result<FindResult> {
    // Step 1: Check current hierarchy before any scrolling.
    let mut root = driver.get_hierarchy().await?;
    let results = find_elements(&root, selector);
    if let Some(found) = results.into_iter().next() {
        return Ok(found);
    }

    let mut direction = initial_direction;
    let mut reversed = false;
    let mut prev_fingerprint = hierarchy_fingerprint(&root);

    for _ in 0..max_scrolls {
        // Step 2: Swipe
        driver.swipe(direction).await?;

        // Step 3: Get new hierarchy and check
        root = driver.get_hierarchy().await?;
        let results = find_elements(&root, selector);
        if let Some(found) = results.into_iter().next() {
            return Ok(found);
        }

        // Step 5: Bounce detection
        let new_fingerprint = hierarchy_fingerprint(&root);
        if new_fingerprint == prev_fingerprint {
            // Hierarchy didn't change -- we've hit the end
            if reversed {
                // Already tried both directions
                bail!(
                    "Element not found: scrolled in both directions and hit boundaries. \
                     Selector: text={:?}, id={:?}, type={:?}",
                    selector.text,
                    selector.id,
                    selector.element_type,
                );
            }
            // Reverse direction and continue
            direction = reverse_direction(direction);
            reversed = true;
        }

        prev_fingerprint = new_fingerprint;
    }

    // Step 7: Max scrolls reached
    bail!(
        "Element not found after {max_scrolls} scroll attempts. \
         Selector: text={:?}, id={:?}, type={:?}",
        selector.text,
        selector.id,
        selector.element_type,
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
            id: None,
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
        Bounds::new(0.0, 0.0, 375.0, 812.0)
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
        fn new(hierarchies: Vec<Element>) -> Self {
            Self {
                hierarchies: Mutex::new(hierarchies),
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

        async fn tap(&self, x: f64, y: f64) -> anyhow::Result<()> {
            self.record_call("tap", vec![x.to_string(), y.to_string()]);
            Ok(())
        }

        async fn long_press(&self, x: f64, y: f64, duration_ms: u64) -> anyhow::Result<()> {
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
            from_x: f64,
            from_y: f64,
            to_x: f64,
            to_y: f64,
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
    }

    // ── 1. Element found in initial hierarchy (no scroll needed) ─────

    #[tokio::test]
    async fn element_found_in_initial_hierarchy() {
        let mut root = make_element("View", default_bounds());
        root.children.push(make_element_with_text(
            "Button",
            "Target",
            Bounds::new(10.0, 10.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert!(swipe_calls.is_empty(), "no swipes should occur");
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
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        // Second hierarchy (after scroll): target appears
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Target",
                Bounds::new(10.0, 100.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Down"]);
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
                    Bounds::new(0.0, 0.0, 200.0, 40.0),
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
        // hierarchy_1: initial (no target)
        // hierarchy_2: same as 1 (bounce!) -- triggers reversal
        // hierarchy_3: different (after reversal, still no target)
        // hierarchy_4: yet different (no target, will exhaust max_scrolls)
        let base = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Static Page",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let different = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Different Page",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let different2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Yet Another Page",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };

        // Sequence: initial check -> same (bounce) -> different -> different2 -> ...
        let driver = SequenceMockDriver::new(vec![
            base.clone(),
            base.clone(), // bounce on first swipe
            different,
            different2,
        ]);
        let selector = sel_with_text("Nonexistent");

        let _ = scroll_to_element(&selector, &driver, Direction::Down, 3).await;

        // Check that swipe direction changed after bounce
        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert!(swipe_calls.len() >= 2);
        // First swipe should be Down, second should be Up (after bounce)
        assert_eq!(swipe_calls[0].1, vec!["Down"]);
        assert_eq!(swipe_calls[1].1, vec!["Up"]);
    }

    // ── 5. Element found after direction reversal ───────────────────

    #[tokio::test]
    async fn element_found_after_direction_reversal() {
        let base = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Bottom Page",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let with_target = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Target",
                Bounds::new(10.0, 100.0, 100.0, 44.0),
            ));
            root
        };

        // initial check: base (no target)
        // after swipe down: base again (bounce!) -> reverse to Up
        // after swipe up: with_target (found!)
        let driver = SequenceMockDriver::new(vec![base.clone(), base.clone(), with_target]);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20)
            .await
            .expect("should find element after direction reversal");

        assert_eq!(result.element.text.as_deref(), Some("Target"));

        let swipe_calls: Vec<_> = driver
            .get_calls()
            .into_iter()
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert_eq!(swipe_calls.len(), 2);
        assert_eq!(swipe_calls[0].1, vec!["Down"]); // first try
        assert_eq!(swipe_calls[1].1, vec!["Up"]); // reversed
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
                    Bounds::new(0.0, 0.0, 200.0, 40.0),
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
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10.0, 10.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Down"]);
    }

    // ── 9. Scroll up direction correct ──────────────────────────────

    #[tokio::test]
    async fn scroll_up_direction_correct() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10.0, 10.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Up"]);
    }

    // ── 10. Scroll left direction works ─────────────────────────────

    #[tokio::test]
    async fn scroll_left_direction_works() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10.0, 10.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Left"]);
    }

    // ── 11. Scroll right direction works ────────────────────────────

    #[tokio::test]
    async fn scroll_right_direction_works() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Page A",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Button",
                "Found",
                Bounds::new(10.0, 10.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
            .collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Right"]);
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
                    Bounds::new(0.0, 0.0, 200.0, 40.0),
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
            .filter(|(m, _)| m == "swipe")
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
                    Bounds::new(0.0, 0.0, 200.0, 40.0),
                ));
                root
            })
            .collect();

        // 4th hierarchy (after 3rd swipe): target found
        let mut target_root = make_element("View", default_bounds());
        target_root.children.push(make_element_with_text(
            "Button",
            "Target",
            Bounds::new(10.0, 10.0, 100.0, 44.0),
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
            .filter(|(m, _)| m == "swipe")
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
                Bounds::new(0.0, 0.0, 200.0, 40.0),
            ));
            root
        };
        let different_page = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text(
                "Label",
                "Different Content",
                Bounds::new(0.0, 0.0, 200.0, 40.0),
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
