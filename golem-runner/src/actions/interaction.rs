use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::{Direction, PlatformDriver};
use golem_parser::Step;
use tokio::time::{sleep, Instant};

use crate::resolution::{build_selector, resolve_element, resolve_element_full_tree};
use crate::scroll::{scroll_to_element, DEFAULT_MAX_SCROLLS};

use super::resolve_element_ignore_text;

const TAP_COOLDOWN: Duration = Duration::from_millis(300);
const DOUBLE_TAP_INTERVAL: Duration = Duration::from_millis(40);

async fn tap_at(driver: &dyn PlatformDriver, x: i32, y: i32) -> Result<()> {
    driver.tap(x, y).await
}

/// Find the target element and tap at its center coordinates.
///
/// When `auto_scroll = true`, if the element is not in the viewport but
/// exists in the full hierarchy, scrolls it into view first. Prefers
/// in-viewport matches when duplicates exist.
///
/// Sleeps 300ms after tapping to prevent accidental double-tap side effects.
pub(crate) async fn handle_tap(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let result = resolve_element(step, driver).await;

    let (x, y) = match result {
        Ok((_elem, coords)) => coords,
        Err(e) if step.auto_scroll.unwrap_or(false) => {
            // Element not in viewport — try auto-scroll.
            // Check the full tree to see if the element exists off-screen.
            let selector = build_selector(step);
            if resolve_element_full_tree(step, driver).await.is_ok() {
                // Element exists off-screen — scroll to it.
                let _found = scroll_to_element(
                    &selector,
                    driver,
                    Direction::Down,
                    DEFAULT_MAX_SCROLLS,
                )
                .await?;
                // After scrolling, re-resolve in viewport to get correct coordinates.
                let (_elem, coords) = resolve_element(step, driver).await?;
                coords
            } else {
                // Element doesn't exist at all — propagate original error.
                return Err(e);
            }
        }
        Err(e) => return Err(e),
    };

    tap_at(driver, x, y).await?;
    sleep(TAP_COOLDOWN).await;
    Ok(())
}

/// Find the target element and double-tap at its center coordinates.
/// Two taps are fired with 40ms between the start of each, followed by a
/// 300ms cooldown.
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
    Ok(())
}

/// Find the target element (input field), tap it to focus, then type text.
///
/// The step's `text` field is the string to type, not an element selector,
/// so we resolve the element using other selectors (id, type, etc.).
pub(crate) async fn handle_type(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element_ignore_text(step, driver).await?;
    driver.tap(x, y).await?;

    let text = step
        .text
        .as_deref()
        .unwrap_or("");
    driver.type_text(text).await
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

    driver.backspace(count).await
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

    driver.long_press(x, y, duration).await
}

/// Swipe in a direction. May optionally target a specific element (ignored for
/// the swipe call itself, but element resolution validates the element exists).
pub(crate) async fn handle_swipe(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let direction_str = step
        .params
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let direction = match direction_str {
        "up" => Direction::Up,
        "down" => Direction::Down,
        "left" => Direction::Left,
        "right" => Direction::Right,
        other => bail!("Invalid swipe direction: \"{}\"", other),
    };

    driver.swipe(direction).await
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

    let max_scrolls = step
        .params
        .get("max_scrolls")
        .and_then(|v| v.as_integer())
        .map(|n| n as u32)
        .unwrap_or(DEFAULT_MAX_SCROLLS);

    let selector = build_selector(step);
    scroll_to_element(&selector, driver, direction, max_scrolls).await?;
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
        step.text = Some("Submit".to_string());

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
        step.text = Some("Submit".to_string());

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
        step.accessibility_id = Some("email".to_string());
        step.text = Some("user@example.com".to_string());

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
        step.accessibility_id = Some("search".to_string());
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
        step.text = Some("Item to select".to_string());
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
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe").collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Up"]);
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
        step.accessibility_id = Some("field".to_string());
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
        step.text = Some("Hold me".to_string());
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

        for (dir_str, expected) in [
            ("up", "Up"),
            ("down", "Down"),
            ("left", "Left"),
            ("right", "Right"),
        ] {
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
            let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe").collect();
            assert_eq!(swipe_calls.len(), 1);
            assert_eq!(swipe_calls[0].1, vec![expected]);
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
        step.text = Some("Does Not Exist".to_string());

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
        step.text = Some("Target".to_string());
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
        step.text = Some("Target".to_string());
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
        step.text = Some("Missing".to_string());
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
        type_step.accessibility_id = Some("username".to_string());
        type_step.text = Some("admin".to_string());
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
        tap_step.text = Some("Login".to_string());
        crate::actions::execute_action(&tap_step, &driver, &mut vars, &ctx, &[])
            .await
            .expect("tap should succeed");

        let calls = driver.get_calls();
        let method_names: Vec<&str> = calls.iter().map(|c| c.0.as_str()).collect();
        // type: get_hierarchy, tap, type_text
        // hide_keyboard: hide_keyboard
        // tap: get_hierarchy, tap
        assert_eq!(
            method_names,
            vec![
                "get_hierarchy",
                "tap",
                "type_text",
                "hide_keyboard",
                "get_hierarchy",
                "tap",
            ]
        );
    }
}
