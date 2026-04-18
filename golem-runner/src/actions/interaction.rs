use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::{Direction, PlatformDriver};
use golem_parser::Step;
use tokio::time::{sleep, Instant};

use crate::resolution::{build_selector, resolve_element, wait_for_settle};
use crate::scroll::{scroll_to_element_with_hint, DEFAULT_MAX_SCROLLS};


/// Minimum delay after a tap to prevent the OS interpreting sequential taps
/// as a double-tap. The settle check runs concurrently but we enforce at
/// least this floor.
const TAP_COOLDOWN: Duration = Duration::from_millis(300);
const DOUBLE_TAP_INTERVAL: Duration = Duration::from_millis(40);

/// If an element is partially off-screen, do a small swipe to bring more of
/// it into view. Useful for `within` containers that are just peeking in.
///
/// Callers should pass effective_bounds (visible_bounds when available).
pub(crate) async fn nudge_into_view(
    driver: &dyn PlatformDriver,
    bounds: &golem_element::Bounds,
    viewport: &golem_element::Viewport,
) {
    let bottom_overflow = (bounds.y + bounds.height) - viewport.height;
    let top_overflow = -(bounds.y);

    // If the container extends significantly below the viewport, swipe up
    // to bring more of it into view
    if bottom_overflow > 50 && bounds.y > 50 {
        let swipe_distance = bottom_overflow.min(viewport.height / 3);
        let cx = viewport.width / 2;
        let cy = viewport.height / 2;
        let _ = driver.swipe_coords(cx, cy + swipe_distance / 2, cx, cy - swipe_distance / 2).await;
        let _ = crate::resolution::wait_for_settle(driver).await;
    }
    // If the container extends above the viewport, swipe down
    else if top_overflow > 50 && bounds.y + bounds.height < viewport.height - 50 {
        let swipe_distance = top_overflow.min(viewport.height / 3);
        let cx = viewport.width / 2;
        let cy = viewport.height / 2;
        let _ = driver.swipe_coords(cx, cy - swipe_distance / 2, cx, cy + swipe_distance / 2).await;
        let _ = crate::resolution::wait_for_settle(driver).await;
    }
}

async fn tap_at(driver: &dyn PlatformDriver, x: i32, y: i32) -> Result<()> {
    driver.tap(x, y).await
}

/// Find a smaller switch/toggle control inside a larger switch/toggle row.
///
/// Spatial matching: looks for a switch/toggle element whose bounds fit entirely
/// inside the outer element AND is positioned on the right half. This handles
/// the iOS SwiftUI Toggle pattern where the label spans the full row but the
/// tappable control is on the right.
async fn find_inner_toggle(
    driver: &dyn PlatformDriver,
    outer: &golem_element::Element,
) -> Option<golem_element::Bounds> {
    let (root, _meta) = driver.get_hierarchy().await.ok()?;
    let mut candidates = Vec::new();
    collect_toggles(&root, &mut candidates);

    let ob = &outer.bounds;
    candidates.into_iter().find(|b| {
        let is_toggle_type = true; // already filtered by collect_toggles
        let fits_inside = b.x >= ob.x
            && b.y >= ob.y
            && b.x + b.width <= ob.x + ob.width
            && b.y + b.height <= ob.y + ob.height;
        let is_smaller = b.width < ob.width || b.height < ob.height;
        let on_right = b.center_x() > ob.center_x();
        is_toggle_type && fits_inside && is_smaller && on_right
    })
}

fn collect_toggles(element: &golem_element::Element, out: &mut Vec<golem_element::Bounds>) {
    let et = element.element_type.to_lowercase();
    if et == "switch" || et == "toggle" {
        out.push(element.bounds.clone());
    }
    for child in &element.children {
        collect_toggles(child, out);
    }
}

/// Find the target element and tap at its center coordinates.
///
/// When `auto_scroll = true`, if the element is not in the viewport but
/// exists in the full hierarchy, scrolls it into view first.
///
/// After tapping, waits for the UI to settle so the next step sees
/// a stable hierarchy.
pub(crate) async fn handle_tap(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (elem, coords) = resolve_element(step, driver).await?;

    // Workaround: iOS SwiftUI Toggles render as a full-width row with the
    // tappable switch control on the right. Tapping the row center hits the
    // label (no effect). We detect this by finding a smaller switch/toggle
    // element that fits inside the matched element's bounds, positioned on
    // the right side. This uses spatial matching (not parent-child) since
    // the visible tree is flat.
    let (x, y) = {
        let et = elem.element_type.to_lowercase();
        if et == "switch" || et == "toggle" {
            if let Some(inner) = find_inner_toggle(driver, &elem).await {
                (inner.center_x(), inner.center_y())
            } else {
                coords
            }
        } else {
            coords
        }
    };

    tap_at(driver, x, y).await?;
    sleep(TAP_COOLDOWN).await;
    let _ = wait_for_settle(driver).await;
    Ok(())
}

/// Find the target element and double-tap at its center coordinates.
/// Two taps are fired with 40ms between the start of each.
pub(crate) async fn handle_double_tap(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;
    let start = Instant::now();
    tap_at(driver, x, y).await?;
    let elapsed = start.elapsed();
    if elapsed < DOUBLE_TAP_INTERVAL {
        sleep(DOUBLE_TAP_INTERVAL - elapsed).await;
    }
    tap_at(driver, x, y).await?;
    sleep(TAP_COOLDOWN).await;
    let _ = wait_for_settle(driver).await;
    Ok(())
}

/// Find the target element (input field), tap it to focus, then type text.
///
/// The `input` field holds the string to type. The `text` field (and other
/// selectors) identify which element to type into.
pub(crate) async fn handle_type(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;
    driver.tap(x, y).await?;

    let value = step
        .input
        .as_deref()
        .or(step.on_text.as_deref())
        .unwrap_or("");
    driver.type_text(value).await?;
    let _ = wait_for_settle(driver).await;
    Ok(())
}

/// Find the target element, tap it to focus, then send backspace key presses.
/// `count` defaults to 1 if not specified in `step.params`.
pub(crate) async fn handle_backspace(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;
    driver.tap(x, y).await?;

    let count = step
        .params
        .get("count")
        .and_then(|v| v.as_integer())
        .map(|n| n as u32)
        .unwrap_or(1);

    driver.backspace(count).await?;
    let _ = wait_for_settle(driver).await;
    Ok(())
}

/// Find the target element and long press at its center coordinates.
/// `duration` in ms, defaults to 1000 if not specified in `step.params`.
pub(crate) async fn handle_long_press(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;

    let duration = step
        .params
        .get("duration")
        .and_then(|v| v.as_integer())
        .map(|n| n as u64)
        .unwrap_or(1000);

    driver.long_press(x, y, duration).await?;
    let _ = wait_for_settle(driver).await;
    Ok(())
}

/// Swipe in a direction. May optionally target a specific element (ignored for
/// the swipe call itself, but element resolution validates the element exists).
pub(crate) async fn handle_swipe(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (root, meta) = driver.get_hierarchy().await?;
    let mut vp = golem_element::Viewport::from_root(&root);
    if meta.keyboard_height > 0 { vp.height -= meta.keyboard_height; }

    // Build the path: start (prepend) + path + end (append)
    let mut path_groups: Vec<&golem_parser::SelectorGroup> = Vec::new();
    if let Some(ref s) = step.start {
        path_groups.push(s);
    }
    for p in &step.points {
        path_groups.push(p);
    }
    if let Some(ref e) = step.end {
        path_groups.push(e);
    }

    // Resolve each path point to (x, y) coordinates
    let resolve_point = |group: &golem_parser::SelectorGroup| -> Option<(i32, i32)> {
        use crate::resolution::build_selector_from_group;
        use golem_element::selector::find_elements;

        let has_element = group.text.is_some() || group.accessibility_label.is_some()
            || group.below.is_some() || group.above.is_some();

        if has_element {
            let sel = build_selector_from_group(group);
            let visible = golem_element::filter_viewport(&root, &vp);
            let results = find_elements(&visible, &sel);
            if let Some(first) = results.first() {
                let base_x = first.element.bounds.center_x();
                let base_y = first.element.bounds.center_y();
                let x = group.x.as_ref().map(|xv| crate::resolution::resolve_coord_public(
                    xv, vp.width, Some(base_x), Some(first.element.bounds.width), Some(first.element.bounds.x)
                )).unwrap_or(base_x);
                let y = group.y.as_ref().map(|yv| crate::resolution::resolve_coord_public(
                    yv, vp.height, Some(base_y), Some(first.element.bounds.height), Some(first.element.bounds.y)
                )).unwrap_or(base_y);
                return Some((x, y));
            }
        }

        if group.x.is_some() || group.y.is_some() {
            let x = group.x.as_ref().map(|xv| crate::resolution::resolve_coord_public(
                xv, vp.width, None, None, None
            )).unwrap_or(vp.width / 2);
            let y = group.y.as_ref().map(|yv| crate::resolution::resolve_coord_public(
                yv, vp.height, None, None, None
            )).unwrap_or(vp.height / 2);
            return Some((x, y));
        }

        None
    };

    let mut points: Vec<(i32, i32)> = path_groups.iter().filter_map(|g| resolve_point(g)).collect();

    // If no path points resolved, use direction to create a 2-point path
    if points.is_empty() {
        let direction_str = step.params.get("direction").and_then(|v| v.as_str()).unwrap_or("");
        let direction = match direction_str {
            "up" => Direction::Up,
            "down" => Direction::Down,
            "left" => Direction::Left,
            "right" => Direction::Right,
            other => bail!("Invalid swipe direction: \"{}\"", other),
        };
        let (sx, sy) = crate::scroll::default_swipe_start(&vp, direction);
        let (fx, fy, tx, ty) = crate::scroll::swipe_from_with_insets(
            &vp, direction, sx, sy, 40,
            meta.safe_area_top, meta.safe_area_bottom.max(meta.keyboard_height),
        );
        points = vec![(fx, fy), (tx, ty)];
    }

    // If only one point + direction, compute the second point
    if points.len() == 1 {
        let direction_str = step.params.get("direction").and_then(|v| v.as_str()).unwrap_or("");
        let dist = vp.height * 2 / 5;
        let (sx, sy) = points[0];
        let end = match direction_str {
            "up" => (sx, sy - dist),
            "down" => (sx, sy + dist),
            "left" => (sx - dist, sy),
            "right" => (sx + dist, sy),
            _ => bail!("swipe with one point requires direction"),
        };
        points.push(end);
    }

    if points.len() < 2 {
        bail!("swipe requires at least 2 points (start + end, or direction)");
    }

    // Execute the swipe — currently only 2-point supported by companion
    if points.len() == 2 {
        driver.swipe_coords(points[0].0, points[0].1, points[1].0, points[1].1).await?;
    } else {
        // Multi-point path (3+ points): not yet fully supported.
        // Chains 2-point swipes which lifts the finger between segments.
        // A continuous multi-point gesture requires a companion /gesture endpoint.
        eprintln!("  [warning] Multi-point path ({} points): finger lifts between segments. Continuous gestures not yet supported.", points.len());
        for window in points.windows(2) {
            driver.swipe_coords(window[0].0, window[0].1, window[1].0, window[1].1).await?;
        }
    }

    let _ = wait_for_settle(driver).await;
    Ok(())
}

/// Scroll in a direction until an element matching the step's selectors is found.
///
/// Params:
/// - `direction`: up/down/left/right (default "down")
/// - `max_scrolls`: optional, defaults to `DEFAULT_MAX_SCROLLS`
pub(crate) async fn handle_scroll(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let direction_str = step
        .params
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("down");

    let direction = match direction_str {
        "up" => Direction::Up,
        "down" => Direction::Down,
        "left" => Direction::Left,
        "right" => Direction::Right,
        other => bail!("Invalid scroll direction: \"{}\"", other),
    };

    let max_scrolls = step.max_scrolls.unwrap_or(DEFAULT_MAX_SCROLLS);

    let selector = build_selector(step);

    // Resolve `within` container — scroll to it first if off-screen.
    let container_bounds = if let Some(ref within_group) = step.within {
        use crate::resolution::build_selector_from_group;
        use golem_element::selector::find_elements;
        let (root, meta) = driver.get_hierarchy().await?;
        let mut vp = golem_element::Viewport::from_root(&root);
        if meta.keyboard_height > 0 { vp.height -= meta.keyboard_height; }
        let visible = golem_element::filter_viewport(&root, &vp);
        let within_sel = build_selector_from_group(within_group);

        let in_viewport = find_elements(&visible, &within_sel)
            .first()
            .map(|r| r.element.bounds.clone());

        if in_viewport.is_some() {
            // Container is visible — but try to get more of it on screen
            nudge_into_view(driver, &in_viewport.clone().expect("checked"), &vp).await;
            // Re-fetch bounds after nudge
            let (r, m) = driver.get_hierarchy().await?;
            let mut v = golem_element::Viewport::from_root(&r);
            if m.keyboard_height > 0 { v.height -= m.keyboard_height; }
            let vis = golem_element::filter_viewport(&r, &v);
            find_elements(&vis, &within_sel)
                .first()
                .map(|r| r.element.bounds.clone())
                .or(in_viewport)
        } else {
            // Container not visible — scroll to bring it into view
            let _ = crate::scroll::scroll_to_element(
                &within_sel, driver, golem_driver::Direction::Down,
                crate::scroll::DEFAULT_MAX_SCROLLS,
            ).await;
            // Nudge to get more of the container visible
            let (fresh, fresh_meta) = driver.get_hierarchy().await?;
            let mut fresh_vp = golem_element::Viewport::from_root(&fresh);
            if fresh_meta.keyboard_height > 0 { fresh_vp.height -= fresh_meta.keyboard_height; }
            let fresh_visible = golem_element::filter_viewport(&fresh, &fresh_vp);
            let bounds = find_elements(&fresh_visible, &within_sel)
                .first()
                .map(|r| r.element.bounds.clone());
            if let Some(ref b) = bounds {
                nudge_into_view(driver, b, &fresh_vp).await;
                // Re-fetch after nudge
                let (r2, m2) = driver.get_hierarchy().await?;
                let mut v2 = golem_element::Viewport::from_root(&r2);
                if m2.keyboard_height > 0 { v2.height -= m2.keyboard_height; }
                let vis2 = golem_element::filter_viewport(&r2, &v2);
                find_elements(&vis2, &within_sel)
                    .first()
                    .map(|r| r.element.bounds.clone())
                    .or(bounds)
            } else {
                bounds
            }
        }
    } else {
        None
    };

    scroll_to_element_with_hint(
        &selector, driver, direction, max_scrolls, 0.0,
        step.scroll_timeout, container_bounds,
    ).await?;
    Ok(())
}

/// Dismiss the on-screen keyboard. No element resolution needed.
pub(crate) async fn handle_hide_keyboard(driver: &dyn PlatformDriver) -> Result<()> {
    driver.hide_keyboard().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use std::path::Path;

    // ── 1. tap action finds element and taps at correct coordinates ──

    #[tokio::test]
    async fn tap_action_finds_element_and_taps_at_correct_coordinates() {
        let root = root_with_button("Submit");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());

        handle_tap(&step, &driver)
            .await
            .expect("tap should succeed");

        let calls = driver.get_calls();
        // get_hierarchy + tap
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1);
        // Button bounds: x=100, y=200, w=100, h=44 => center = (150, 222)
        assert_eq!(tap_calls[0].1, vec!["150", "222"]);
    }

    // ── 2. doubleTap sends two taps at correct coordinates ───────────

    #[tokio::test]
    async fn double_tap_sends_two_taps_at_correct_coordinates() {
        let root = root_with_button("Submit");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("doubleTap");
        step.on_text = Some("Submit".to_string());

        handle_double_tap(&step, &driver)
            .await
            .expect("doubleTap SHALL succeed");

        let calls = driver.get_calls();
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 2, "doubleTap SHALL produce exactly two tap calls");
        // Both taps hit the same center: x=100+100/2=150, y=200+44/2=222
        assert_eq!(tap_calls[0].1, vec!["150", "222"]);
        assert_eq!(tap_calls[1].1, vec!["150", "222"]);
    }

    // ── 3. type action types text to element ─────────────────────────

    #[tokio::test]
    async fn type_action_types_text_to_element() {
        let root = root_with_input("email");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("type");
        step.on_accessibility_label = Some("email".to_string());
        step.input = Some("user@example.com".to_string());

        handle_type(&step, &driver)
            .await
            .expect("type should succeed");

        let calls = driver.get_calls();
        // Should have: get_hierarchy, tap (to focus), type_text
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1);
        // TextField bounds: x=20, y=100, w=300, h=44 => center = (170, 122)
        assert_eq!(tap_calls[0].1, vec!["170", "122"]);

        let type_calls: Vec<_> = calls.iter().filter(|c| c.0 == "type_text").collect();
        assert_eq!(type_calls.len(), 1);
        assert_eq!(type_calls[0].1, vec!["user@example.com"]);
    }

    // ── 4. backspace action with count ───────────────────────────────

    #[tokio::test]
    async fn backspace_action_with_count() {
        let root = root_with_input("search");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("backspace");
        step.on_accessibility_label = Some("search".to_string());
        step.params
            .insert("count".to_string(), toml::Value::Integer(5));

        handle_backspace(&step, &driver)
            .await
            .expect("backspace should succeed");

        let calls = driver.get_calls();
        let bs_calls: Vec<_> = calls.iter().filter(|c| c.0 == "backspace").collect();
        assert_eq!(bs_calls.len(), 1);
        assert_eq!(bs_calls[0].1, vec!["5"]);
    }

    // ── 5. long_press action at element coordinates ──────────────────

    #[tokio::test]
    async fn long_press_action_at_element_coordinates() {
        let root = root_with_button("Item to select");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("long_press");
        step.on_text = Some("Item to select".to_string());
        step.params
            .insert("duration".to_string(), toml::Value::Integer(2000));

        handle_long_press(&step, &driver)
            .await
            .expect("long_press should succeed");

        let calls = driver.get_calls();
        let lp_calls: Vec<_> = calls.iter().filter(|c| c.0 == "long_press").collect();
        assert_eq!(lp_calls.len(), 1);
        // Button center = (150, 222), duration = 2000
        assert_eq!(lp_calls[0].1, vec!["150", "222", "2000"]);
    }

    // ── 6. swipe action with direction ───────────────────────────────

    #[tokio::test]
    async fn swipe_action_with_direction() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.params
            .insert("direction".to_string(), toml::Value::String("up".to_string()));

        handle_swipe(&step, &driver)
            .await
            .expect("swipe should succeed");

        let calls = driver.get_calls();
        // Now uses swipe_coords instead of swipe(direction)
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), 1);
    }

    // ── 7. hide_keyboard action ──────────────────────────────────────

    #[tokio::test]
    async fn hide_keyboard_action() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("hide_keyboard");

        handle_hide_keyboard(&driver)
            .await
            .expect("hide_keyboard should succeed");

        let calls = driver.get_calls();
        let hk_calls: Vec<_> = calls.iter().filter(|c| c.0 == "hide_keyboard").collect();
        assert_eq!(hk_calls.len(), 1);
        let _ = step; // suppress unused warning
    }

    // ── Extra: backspace defaults count to 1 ─────────────────────────

    #[tokio::test]
    async fn backspace_defaults_count_to_one() {
        let root = root_with_input("field");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("backspace");
        step.on_accessibility_label = Some("field".to_string());
        // No count param set

        handle_backspace(&step, &driver)
            .await
            .expect("backspace should succeed");

        let calls = driver.get_calls();
        let bs_calls: Vec<_> = calls.iter().filter(|c| c.0 == "backspace").collect();
        assert_eq!(bs_calls.len(), 1);
        assert_eq!(bs_calls[0].1, vec!["1"]);
    }

    // ── Extra: long_press defaults duration to 1000 ──────────────────

    #[tokio::test]
    async fn long_press_defaults_duration_to_1000() {
        let root = root_with_button("Hold me");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("long_press");
        step.on_text = Some("Hold me".to_string());
        // No duration param set

        handle_long_press(&step, &driver)
            .await
            .expect("long_press should succeed");

        let calls = driver.get_calls();
        let lp_calls: Vec<_> = calls.iter().filter(|c| c.0 == "long_press").collect();
        assert_eq!(lp_calls.len(), 1);
        assert_eq!(lp_calls[0].1[2], "1000");
    }

    // ── Extra: swipe with all four directions ────────────────────────

    #[tokio::test]
    async fn swipe_all_directions() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        for dir_str in ["up", "down", "left", "right"] {
            driver.clear_calls();
            let mut step = make_step("swipe");
            step.params.insert(
                "direction".to_string(),
                toml::Value::String(dir_str.to_string()),
            );

            handle_swipe(&step, &driver)
                .await
                .unwrap_or_else(|_| panic!("swipe {dir_str} should succeed"));

            let calls = driver.get_calls();
            let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
            assert_eq!(swipe_calls.len(), 1, "swipe {dir_str} should produce one swipe_coords call");
        }
    }

    // ── Extra: swipe with invalid direction returns error ────────────

    #[tokio::test]
    async fn swipe_invalid_direction_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.params.insert(
            "direction".to_string(),
            toml::Value::String("diagonal".to_string()),
        );

        let result = handle_swipe(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Invalid swipe direction"),
            "error should mention invalid direction, got: {err_msg}"
        );
    }

    // ── tap on non-existent element returns error ────────────────

    #[tokio::test]
    async fn tap_on_nonexistent_element_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("tap");
        step.on_text = Some("Does Not Exist".to_string());

        let result = handle_tap(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No element found"),
            "error should mention no element found, got: {err_msg}"
        );
    }

    // ── scroll action dispatches to scroll_to_element ─────────────────

    #[tokio::test]
    async fn scroll_action_dispatches_with_direction() {
        // Hierarchy that already contains the target, so scroll returns immediately.
        let root = root_with_button("Target");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("scroll");
        step.on_text = Some("Target".to_string());
        step.params.insert(
            "direction".to_string(),
            toml::Value::String("up".to_string()),
        );

        handle_scroll(&step, &driver)
            .await
            .expect("scroll SHALL succeed when element is already visible");

        // Element found immediately -- no swipe calls expected
        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe").collect();
        assert!(
            swipe_calls.is_empty(),
            "no swipes SHALL occur when element is found immediately"
        );
    }

    #[tokio::test]
    async fn scroll_action_uses_default_direction_down() {
        let root = root_with_button("Target");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("scroll");
        step.on_text = Some("Target".to_string());
        // No direction param -- should default to "down"

        handle_scroll(&step, &driver)
            .await
            .expect("scroll SHALL succeed with default direction");
    }

    #[tokio::test]
    async fn scroll_action_uses_custom_max_scrolls() {
        // Empty hierarchy -- element never found
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("scroll");
        step.on_text = Some("Missing".to_string());
        step.params.insert(
            "max_scrolls".to_string(),
            toml::Value::Integer(2),
        );

        let result = handle_scroll(&step, &driver).await;
        assert!(result.is_err(), "scroll SHALL fail when element not found");
    }

    // ── 8. multiple actions in sequence ──────────────────────────────

    #[tokio::test]
    async fn multiple_actions_in_sequence() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_id(
            "TextField",
            "username",
            Bounds::new(20, 100, 300, 44),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Login",
            Bounds::new(100, 200, 100, 44),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        // Type into username field
        let mut type_step = make_step("type");
        type_step.on_accessibility_label = Some("username".to_string());
        type_step.input = Some("admin".to_string());
        crate::actions::execute_action(&type_step, &driver, &mut vars, &ctx, &[])
            .await
            .expect("type should succeed");

        // Hide keyboard
        let hk_step = make_step("hide_keyboard");
        crate::actions::execute_action(&hk_step, &driver, &mut vars, &ctx, &[])
            .await
            .expect("hide_keyboard should succeed");

        // Tap login button
        let mut tap_step = make_step("tap");
        tap_step.on_text = Some("Login".to_string());
        crate::actions::execute_action(&tap_step, &driver, &mut vars, &ctx, &[])
            .await
            .expect("tap should succeed");

        let calls = driver.get_calls();
        let method_names: Vec<&str> = calls.iter().map(|c| c.0.as_str()).collect();
        // type: get_hierarchy (resolve), tap, type_text, get_hierarchy x2 (settle)
        // hide_keyboard: hide_keyboard
        // tap: get_hierarchy (resolve), tap, get_hierarchy x2 (settle)
        assert_eq!(
            method_names,
            vec![
                "get_hierarchy",
                "tap",
                "type_text",
                "get_hierarchy", "get_hierarchy",
                "hide_keyboard",
                "get_hierarchy",
                "tap",
                "get_hierarchy", "get_hierarchy",
            ]
        );
    }
}
