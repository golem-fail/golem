use std::time::Duration;

use anyhow::Result;
use golem_driver::{Direction, PlatformDriver};
use golem_parser::Step;
use tokio::time::{sleep, Instant};

use crate::context::ExecutionContext;
use crate::resolution::{build_selector, resolve_element};


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
    let (root, _meta) = crate::resolution::get_hierarchy_bounded(driver).await.ok()?;
    let mut candidates = Vec::new();
    collect_toggles(&root, &mut candidates);

    // iOS occasionally reports a child element's frame a few points larger
    // than its parent (SwiftUI hit-target rounding). Without a tolerance,
    // strict containment rejects the very pair this workaround targets.
    const TOL: i32 = 4;
    let ob = &outer.bounds;
    candidates.into_iter().find(|b| {
        let fits_inside = b.x + TOL >= ob.x
            && b.y + TOL >= ob.y
            && b.x + b.width <= ob.x + ob.width + TOL
            && b.y + b.height <= ob.y + ob.height + TOL;
        let is_smaller = b.width < ob.width || b.height < ob.height;
        let on_right = b.center_x() > ob.center_x();
        fits_inside && is_smaller && on_right
    })
}

fn collect_toggles(element: &golem_element::Element, out: &mut Vec<golem_element::Bounds>) {
    let et = element.element_type.to_lowercase();
    if et == "switch" || et == "toggle" {
        out.push(element.bounds);
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
pub(crate) async fn handle_tap(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    let (elem, coords) = resolve_element(step, driver, ctx.emitter).await?;

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

    ctx.substep(golem_events::SubstepEvent::Tap {
        point: golem_events::Point { x, y },
        element_bounds: Some(golem_events::Rect {
            x: elem.bounds.x, y: elem.bounds.y,
            width: elem.bounds.width, height: elem.bounds.height,
        }),
    });
    tap_at(driver, x, y).await?;
    sleep(TAP_COOLDOWN).await;
    Ok(())
}

/// Find the target element and double-tap at its center coordinates.
/// Two taps are fired with 40ms between the start of each.
pub(crate) async fn handle_double_tap(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver, ctx.emitter).await?;
    ctx.substep(golem_events::SubstepEvent::DoubleTap {
        point: golem_events::Point { x, y },
        element_bounds: None,
    });
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
/// The `input` field holds the string to type. The `text` field (and other
/// selectors) identify which element to type into.
pub(crate) async fn handle_type(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    let value = step
        .input
        .as_deref()
        .or(step.on_text.as_deref())
        .unwrap_or("");

    // Tap to focus, then verify focus before typing. Under heavy load
    // or mid-animation (keyboard opening from a prior step), the tap
    // can land on the keyboard's top edge → field loses focus → the
    // keystrokes drop into nothing. Re-resolve + retry once if the
    // element isn't focused after the tap.
    let selector = build_selector(step);
    let mut attempts = 0;
    loop {
        let (_elem, (x, y)) = resolve_element(step, driver, ctx.emitter).await?;
        driver.tap(x, y).await?;

        // Give the keyboard a moment to finish opening before checking
        // focus. Keyboard animations on Android are typically 200-400ms.
        sleep(Duration::from_millis(400)).await;

        let (root, _meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
        let matches = golem_element::selector::find_elements(&root, &selector);
        let now_focused = matches.iter().any(|r| r.element.focused);
        if now_focused || attempts >= 1 {
            break;
        }
        attempts += 1;
    }

    ctx.substep(golem_events::SubstepEvent::TextInput {
        text: value.to_string(),
        field_bounds: None,
    });
    driver.type_text(value).await?;
    Ok(())
}

/// Find the target element, tap it to focus, then send backspace key presses.
/// `count` defaults to 1 if not specified in `step.params`.
pub(crate) async fn handle_backspace(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver, ctx.emitter).await?;
    driver.tap(x, y).await?;

    let count = step
        .params
        .get("count")
        .and_then(|v| v.as_integer())
        .map(|n| n as u32)
        .unwrap_or(1);

    driver.backspace(count).await?;
    Ok(())
}

/// Find the target element and long press at its center coordinates.
/// `duration` in ms, defaults to 1000 if not specified in `step.params`.
pub(crate) async fn handle_long_press(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver, ctx.emitter).await?;

    let duration = step.duration.unwrap_or(1000);

    driver.long_press(x, y, duration).await?;
    Ok(())
}

/// Swipe in a direction. May optionally target a specific element (ignored for
/// the swipe call itself, but element resolution validates the element exists).
pub(crate) async fn handle_swipe(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
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
            other => crate::fail_code!(golem_events::FailureCode::ParseMissingParam, "Invalid swipe direction: \"{}\"", other),
        };
        let safe_vp = crate::scroll::make_safe_viewport(&vp, &meta);
        let (sx, sy) = crate::scroll::default_swipe_start(&safe_vp, direction);
        let (fx, fy, tx, ty) = crate::scroll::swipe_from(&safe_vp, direction, sx, sy, 40);
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
            _ => crate::fail_code!(golem_events::FailureCode::ParseMissingParam, "swipe with one point requires direction"),
        };
        points.push(end);
    }

    if points.len() < 2 {
        crate::fail_code!(golem_events::FailureCode::ParseMissingParam, "swipe requires at least 2 points (start + end, or direction)");
    }

    // Execute the swipe
    ctx.substep(golem_events::SubstepEvent::Swipe {
        from: golem_events::Point { x: points[0].0, y: points[0].1 },
        to: golem_events::Point { x: points.last().map_or(0, |p| p.0), y: points.last().map_or(0, |p| p.1) },
        duration_ms: None,
    });
    if points.len() == 2 {
        driver.swipe_coords(points[0].0, points[0].1, points[1].0, points[1].1).await?;
    } else {
        // 3+ points: continuous gesture (single finger, no lift between segments)
        let duration = step.duration.unwrap_or(300);
        driver.gesture(vec![golem_driver::GestureFinger {
            points,
            duration_ms: duration,
        }]).await?;
    }

    Ok(())
}

/// Scroll in a direction until an element matching the step's selectors is found.
///
/// Params:
/// - `direction`: up/down/left/right (default "down")
///
/// Termination: action timeout bounds wall-clock; stall detection bails
/// when consecutive swipes have no effect on the tree. Number of swipes
/// is unbounded by design.
pub(crate) async fn handle_scroll(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
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
        other => crate::fail_code!(golem_events::FailureCode::ParseMissingParam, "Invalid scroll direction: \"{}\"", other),
    };

    let selector = build_selector(step);

    // Resolve `within` container — scroll to it first if off-screen.
    let container_bounds = if let Some(ref within_group) = step.within {
        use crate::resolution::build_selector_from_group;
        use golem_element::selector::find_elements;
        let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
        let mut vp = golem_element::Viewport::from_root(&root);
        if meta.keyboard_height > 0 { vp.height -= meta.keyboard_height; }
        let visible = golem_element::filter_viewport(&root, &vp);
        let within_sel = build_selector_from_group(within_group);

        let in_viewport = find_elements(&visible, &within_sel)
            .first()
            .map(|r| r.element.bounds);

        if let Some(b) = in_viewport {
            // Container is visible — but try to get more of it on screen
            nudge_into_view(driver, &b, &vp).await;
            // Re-fetch bounds after nudge
            let (r, m) = crate::resolution::get_hierarchy_bounded(driver).await?;
            let mut v = golem_element::Viewport::from_root(&r);
            if m.keyboard_height > 0 { v.height -= m.keyboard_height; }
            let vis = golem_element::filter_viewport(&r, &v);
            find_elements(&vis, &within_sel)
                .first()
                .map(|r| r.element.bounds)
                .or(in_viewport)
        } else {
            // Container not visible — scroll to bring it into view
            let _ = crate::scroll::scroll_to_element(
                &within_sel, driver, golem_driver::Direction::Down,
                None, None, ctx.emitter,
            ).await;
            // Nudge to get more of the container visible
            let (fresh, fresh_meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
            let mut fresh_vp = golem_element::Viewport::from_root(&fresh);
            if fresh_meta.keyboard_height > 0 { fresh_vp.height -= fresh_meta.keyboard_height; }
            let fresh_visible = golem_element::filter_viewport(&fresh, &fresh_vp);
            let bounds = find_elements(&fresh_visible, &within_sel)
                .first()
                .map(|r| r.element.bounds);
            if let Some(ref b) = bounds {
                nudge_into_view(driver, b, &fresh_vp).await;
                // Re-fetch after nudge
                let (r2, m2) = crate::resolution::get_hierarchy_bounded(driver).await?;
                let mut v2 = golem_element::Viewport::from_root(&r2);
                if m2.keyboard_height > 0 { v2.height -= m2.keyboard_height; }
                let vis2 = golem_element::filter_viewport(&r2, &v2);
                find_elements(&vis2, &within_sel)
                    .first()
                    .map(|r| r.element.bounds)
                    .or(bounds)
            } else {
                bounds
            }
        }
    } else {
        None
    };

    crate::scroll::scroll_to_element(
        &selector, driver, direction,
        step.scroll_timeout, container_bounds, ctx.emitter,
    ).await?;
    Ok(())
}

/// Dismiss the on-screen keyboard. No element resolution needed.
pub(crate) async fn handle_hide_keyboard(driver: &dyn PlatformDriver) -> Result<()> {
    driver.hide_keyboard().await
}

/// Resolve a coordinate from a TOML param value: integer pixels or "N%" string.
fn resolve_param_coord(val: Option<&toml::Value>, viewport_size: i32) -> i32 {
    match val {
        Some(toml::Value::Integer(n)) => *n as i32,
        Some(toml::Value::String(s)) if s.ends_with('%') => {
            if let Ok(pct) = s.trim_end_matches('%').parse::<f64>() {
                (viewport_size as f64 * pct / 100.0) as i32
            } else {
                viewport_size / 2
            }
        }
        _ => viewport_size / 2,
    }
}

/// Resolve center coordinates for a gesture step.
///
/// If the step has element selectors, resolves the element and uses its center.
/// Otherwise falls back to x/y params or viewport center.
/// Uses a single hierarchy fetch for both viewport and element resolution.
async fn resolve_gesture_center(step: &Step, driver: &dyn PlatformDriver) -> Result<(i32, i32)> {
    let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
    let mut vp = golem_element::Viewport::from_root(&root);
    if meta.keyboard_height > 0 { vp.height -= meta.keyboard_height; }

    let has_selector = step.on_text.is_some() || step.on.is_some()
        || step.on_accessibility_label.is_some();

    if has_selector {
        let sel = build_selector(step);
        let visible = golem_element::filter_viewport(&root, &vp);
        let results = golem_element::selector::find_elements(&visible, &sel);
        if let Some(first) = results.first() {
            let eb = first.element.effective_bounds();
            Ok((eb.center_x(), eb.center_y()))
        } else {
            crate::fail_code!(golem_events::FailureCode::FlowElementNotFound, "No element found matching selector");
        }
    } else {
        let x = resolve_param_coord(step.params.get("x"), vp.width);
        let y = resolve_param_coord(step.params.get("y"), vp.height);
        Ok((x, y))
    }
}

/// Pinch gesture on an element or at coordinates.
///
/// Params (from step fields):
/// - `scale` (required): >1.0 = zoom in, <1.0 = zoom out
/// - `velocity` (optional, default 5.0): scale factor per second
/// - Element selector or x/y coordinates for center point
pub(crate) async fn handle_pinch(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let scale = step.scale.ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("pinch requires 'scale' parameter")))?;
    let velocity = step.velocity.unwrap_or(5.0);
    let (cx, cy) = resolve_gesture_center(step, driver).await?;

    driver.pinch(cx, cy, scale, velocity).await?;
    Ok(())
}

/// Rotate gesture on an element or at coordinates.
///
/// Two fingers orbit around the center point by the given angle.
/// Positive = clockwise, negative = counter-clockwise.
///
/// Params (from step fields):
/// - `rotation` (required): degrees to rotate (positive = CW, negative = CCW)
/// - `velocity` (optional, default 180.0): degrees per second
/// - Element selector or x/y coordinates for center point
pub(crate) async fn handle_rotate_gesture(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let degrees = step.rotation.ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("rotate requires 'rotation' parameter")))?;
    let velocity = step.velocity.unwrap_or(180.0);
    let (cx, cy) = resolve_gesture_center(step, driver).await?;

    // Compute two-finger circular arc paths
    let radius = 50.0_f64; // 50px orbit radius
    let duration_ms = (degrees.abs() / velocity * 1000.0).max(200.0) as u64;
    let steps = ((degrees.abs() / 10.0).ceil() as usize).max(3); // ~1 point per 10°
    let radians = degrees.to_radians();
    let cx = cx as f64;
    let cy = cy as f64;

    let mut finger1 = Vec::new();
    let mut finger2 = Vec::new();
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let angle = t * radians;
        // Finger 1 starts at top (angle 0 = -PI/2 from x-axis)
        let a1 = -std::f64::consts::FRAC_PI_2 + angle;
        finger1.push((
            (cx + radius * a1.cos()) as i32,
            (cy + radius * a1.sin()) as i32,
        ));
        // Finger 2 is opposite (180° offset)
        let a2 = std::f64::consts::FRAC_PI_2 + angle;
        finger2.push((
            (cx + radius * a2.cos()) as i32,
            (cy + radius * a2.sin()) as i32,
        ));
    }

    driver.gesture(vec![
        golem_driver::GestureFinger { points: finger1, duration_ms },
        golem_driver::GestureFinger { points: finger2, duration_ms },
    ]).await?;
    Ok(())
}

/// Arbitrary multi-touch gesture with explicit finger paths.
///
/// Each finger in step.fingers has `points` (Vec<SelectorGroup>) resolved to coordinates.
pub(crate) async fn handle_gesture(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    if step.fingers.is_empty() {
        crate::fail_code!(golem_events::FailureCode::ParseMissingParam, "gesture requires at least one finger in 'fingers' array");
    }

    let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
    let mut vp = golem_element::Viewport::from_root(&root);
    if meta.keyboard_height > 0 { vp.height -= meta.keyboard_height; }

    let duration = step.duration.unwrap_or(300);

    let mut gesture_fingers = Vec::new();
    for finger in &step.fingers {
        if finger.points.len() < 2 {
            crate::fail_code!(golem_events::FailureCode::ParseMissingParam, "each finger needs at least 2 points");
        }
        let mut points = Vec::new();
        for group in &finger.points {
            let has_element = group.text.is_some() || group.accessibility_label.is_some()
                || group.below.is_some() || group.above.is_some();

            if has_element {
                let sel = crate::resolution::build_selector_from_group(group);
                let visible = golem_element::filter_viewport(&root, &vp);
                let results = golem_element::selector::find_elements(&visible, &sel);
                if let Some(first) = results.first() {
                    let base_x = first.element.effective_bounds().center_x();
                    let base_y = first.element.effective_bounds().center_y();
                    let w = first.element.effective_bounds().width;
                    let h = first.element.effective_bounds().height;
                    let x = group.x.as_ref().map(|xv| crate::resolution::resolve_coord_public(
                        xv, vp.width, Some(base_x), Some(w), Some(first.element.effective_bounds().x)
                    )).unwrap_or(base_x);
                    let y = group.y.as_ref().map(|yv| crate::resolution::resolve_coord_public(
                        yv, vp.height, Some(base_y), Some(h), Some(first.element.effective_bounds().y)
                    )).unwrap_or(base_y);
                    points.push((x, y));
                    continue;
                }
            }

            let x = group.x.as_ref().map(|xv| crate::resolution::resolve_coord_public(
                xv, vp.width, None, None, None
            )).unwrap_or(vp.width / 2);
            let y = group.y.as_ref().map(|yv| crate::resolution::resolve_coord_public(
                yv, vp.height, None, None, None
            )).unwrap_or(vp.height / 2);
            points.push((x, y));
        }
        gesture_fingers.push(golem_driver::GestureFinger {
            points,
            duration_ms: duration,
        });
    }

    driver.gesture(gesture_fingers).await?;
    Ok(())
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

        let ctx = test_ctx(Path::new("."));
        handle_tap(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_double_tap(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_type(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_backspace(&step, &driver, &ctx)
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
        step.duration = Some(2000);

        let ctx = test_ctx(Path::new("."));
        handle_long_press(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_swipe(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_backspace(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_long_press(&step, &driver, &ctx)
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

            let ctx = test_ctx(Path::new("."));
        handle_swipe(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        let result = handle_swipe(&step, &driver, &ctx).await;
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
        // Tight test-only timeout: tap polls for the element until the
        // deadline; we just want fast failure.
        step.timeout = Some(50);

        let ctx = test_ctx(Path::new("."));
        let result = handle_tap(&step, &driver, &ctx).await;
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

        let ctx = test_ctx(Path::new("."));
        handle_scroll(&step, &driver, &ctx)
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

        let ctx = test_ctx(Path::new("."));
        handle_scroll(&step, &driver, &ctx)
            .await
            .expect("scroll SHALL succeed with default direction");
    }

    // ── 8. multiple actions in sequence ──────────────────────────────

    #[tokio::test]
    async fn multiple_actions_in_sequence() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut input = make_element_with_id(
            "TextField",
            "username",
            Bounds::new(20, 100, 300, 44),
        );
        input.focused = true;
        root.children.push(input);
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
        // execute_action bypasses execute_step_with_policy, so the
        // out-of-band post-settle (added in policy.rs) doesn't run here.
        // type: get_hierarchy (resolve), tap (focus), get_hierarchy
        //   (post-tap focus check), type_text
        // hide_keyboard: hide_keyboard
        // tap: get_hierarchy (resolve), tap
        assert_eq!(
            method_names,
            vec![
                "get_hierarchy",
                "tap",
                "get_hierarchy",
                "type_text",
                "hide_keyboard",
                "get_hierarchy",
                "tap",
            ]
        );
    }

    // ── 9. resolve_param_coord: integer pixels passed through ─────────

    #[test]
    fn resolve_param_coord_integer_is_passed_through() {
        let val = toml::Value::Integer(123);
        assert_eq!(
            resolve_param_coord(Some(&val), 1000),
            123,
            "integer param SHALL be returned verbatim as pixels"
        );
    }

    // ── 10. resolve_param_coord: valid percentage of viewport ─────────

    #[test]
    fn resolve_param_coord_percentage_is_fraction_of_viewport() {
        let val = toml::Value::String("25%".to_string());
        assert_eq!(
            resolve_param_coord(Some(&val), 800),
            200,
            "'25%' SHALL resolve to a quarter of the viewport size"
        );
    }

    // ── 11. resolve_param_coord: malformed percentage falls back to center ──

    #[test]
    fn resolve_param_coord_bad_percentage_falls_back_to_center() {
        let val = toml::Value::String("abc%".to_string());
        assert_eq!(
            resolve_param_coord(Some(&val), 600),
            300,
            "unparsable percentage SHALL fall back to viewport center"
        );
    }

    // ── 12. resolve_param_coord: None / non-coord types → center ──────

    #[test]
    fn resolve_param_coord_none_and_other_types_fall_back_to_center() {
        assert_eq!(
            resolve_param_coord(None, 400),
            200,
            "missing param SHALL fall back to viewport center"
        );
        // A non-percent string (no trailing '%') hits the catch-all arm.
        let plain = toml::Value::String("nope".to_string());
        assert_eq!(
            resolve_param_coord(Some(&plain), 400),
            200,
            "non-percent string SHALL fall back to viewport center"
        );
        // A boolean (neither Integer nor String) hits the catch-all arm.
        let b = toml::Value::Boolean(true);
        assert_eq!(
            resolve_param_coord(Some(&b), 400),
            200,
            "non-coordinate type SHALL fall back to viewport center"
        );
    }

    // ── 13. collect_toggles gathers switch/toggle bounds recursively ──

    #[test]
    fn collect_toggles_gathers_switches_and_toggles_recursively() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let switch = make_element("Switch", Bounds::new(300, 100, 40, 24));
        let mut group = make_element("View", Bounds::new(0, 200, 375, 60));
        let toggle = make_element("Toggle", Bounds::new(320, 210, 40, 24));
        group.children.push(toggle);
        let plain = make_element("Button", Bounds::new(10, 400, 100, 44));
        root.children.push(switch);
        root.children.push(group);
        root.children.push(plain);

        let mut out = Vec::new();
        collect_toggles(&root, &mut out);
        assert_eq!(
            out.len(),
            2,
            "collect_toggles SHALL find both the top-level switch and the nested toggle"
        );
        assert!(
            out.iter().any(|b| b.x == 300),
            "the top-level switch bounds SHALL be collected"
        );
        assert!(
            out.iter().any(|b| b.x == 320),
            "the nested toggle bounds SHALL be collected"
        );
    }

    // ── 14. tap on a Switch row retargets to the inner toggle control ──

    #[tokio::test]
    async fn tap_on_switch_row_retargets_to_inner_toggle() {
        // Outer Switch row spans full width; a smaller inner Switch sits on
        // the right half. find_inner_toggle SHALL pick the inner control.
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut outer = make_element_with_text("Switch", "Wifi", Bounds::new(0, 200, 375, 60));
        let inner = make_element("Switch", Bounds::new(320, 218, 40, 24));
        outer.children.push(inner);
        root.children.push(outer);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("tap");
        step.on_text = Some("Wifi".to_string());

        let ctx = test_ctx(Path::new("."));
        handle_tap(&step, &driver, &ctx)
            .await
            .expect("tap on switch SHALL succeed");

        let calls = driver.get_calls();
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1);
        // Inner toggle center = (320 + 40/2, 218 + 24/2) = (340, 230)
        assert_eq!(
            tap_calls[0].1,
            vec!["340", "230"],
            "tap SHALL retarget to the inner toggle's center, not the row center"
        );
    }

    // ── 15. tap on a Switch with no inner control taps the matched center ──

    #[tokio::test]
    async fn tap_on_switch_without_inner_toggle_uses_matched_center() {
        // A lone Switch with no smaller inner control: find_inner_toggle
        // returns None, so the tap falls back to the matched coords.
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let outer = make_element_with_text("Switch", "Alone", Bounds::new(100, 200, 100, 44));
        root.children.push(outer);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("tap");
        step.on_text = Some("Alone".to_string());

        let ctx = test_ctx(Path::new("."));
        handle_tap(&step, &driver, &ctx)
            .await
            .expect("tap on lone switch SHALL succeed");

        let calls = driver.get_calls();
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1);
        // Matched center = (100 + 100/2, 200 + 44/2) = (150, 222)
        assert_eq!(
            tap_calls[0].1,
            vec!["150", "222"],
            "with no inner toggle the tap SHALL hit the matched element center"
        );
    }

    // ── 16. pinch without scale returns ParseMissingParam error ───────

    #[tokio::test]
    async fn pinch_without_scale_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("pinch"); // no scale set

        let result = handle_pinch(&step, &driver).await;
        assert!(result.is_err(), "pinch without scale SHALL error");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("scale"),
            "error SHALL mention the missing scale param, got: {err_msg}"
        );
    }

    // ── 17. pinch at x/y params uses those coords and default velocity ──

    #[tokio::test]
    async fn pinch_with_coords_uses_params_and_default_velocity() {
        let root = make_element("View", Bounds::new(0, 0, 800, 600));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("pinch");
        step.scale = Some(2.0);
        step.params.insert("x".to_string(), toml::Value::Integer(123));
        step.params.insert("y".to_string(), toml::Value::Integer(456));
        // No velocity -> default 5.0

        handle_pinch(&step, &driver)
            .await
            .expect("pinch SHALL succeed");

        let calls = driver.get_calls();
        let pinch_calls: Vec<_> = calls.iter().filter(|c| c.0 == "pinch").collect();
        assert_eq!(pinch_calls.len(), 1);
        // mock records [x, y, scale, velocity]
        assert_eq!(pinch_calls[0].1[0], "123");
        assert_eq!(pinch_calls[0].1[1], "456");
        assert_eq!(pinch_calls[0].1[2], "2");
        assert_eq!(
            pinch_calls[0].1[3], "5",
            "velocity SHALL default to 5.0 when unset"
        );
    }

    // ── 18. pinch centered on a resolved element ──────────────────────

    #[tokio::test]
    async fn pinch_on_element_uses_element_center() {
        let root = root_with_button("Map");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("pinch");
        step.scale = Some(0.5);
        step.velocity = Some(3.0);
        step.on_text = Some("Map".to_string());

        handle_pinch(&step, &driver)
            .await
            .expect("pinch on element SHALL succeed");

        let calls = driver.get_calls();
        let pinch_calls: Vec<_> = calls.iter().filter(|c| c.0 == "pinch").collect();
        assert_eq!(pinch_calls.len(), 1);
        // Button center = (150, 222)
        assert_eq!(pinch_calls[0].1[0], "150");
        assert_eq!(pinch_calls[0].1[1], "222");
        assert_eq!(pinch_calls[0].1[3], "3");
    }

    // ── 19. pinch on a missing element selector errors ────────────────

    #[tokio::test]
    async fn pinch_on_missing_element_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("pinch");
        step.scale = Some(2.0);
        step.on_text = Some("Nope".to_string());

        let result = handle_pinch(&step, &driver).await;
        assert!(result.is_err(), "pinch on missing element SHALL error");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No element found"),
            "error SHALL mention no element found, got: {err_msg}"
        );
    }

    // ── 20. rotate without rotation param returns error ───────────────

    #[tokio::test]
    async fn rotate_without_rotation_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("rotate"); // no rotation set

        let result = handle_rotate_gesture(&step, &driver).await;
        assert!(result.is_err(), "rotate without rotation SHALL error");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("rotation"),
            "error SHALL mention the missing rotation param, got: {err_msg}"
        );
    }

    // ── 21. rotate emits a two-finger gesture ─────────────────────────

    #[tokio::test]
    async fn rotate_emits_two_finger_gesture() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("rotate");
        step.rotation = Some(90.0);
        step.params.insert("x".to_string(), toml::Value::Integer(100));
        step.params.insert("y".to_string(), toml::Value::Integer(100));

        handle_rotate_gesture(&step, &driver)
            .await
            .expect("rotate SHALL succeed");

        let calls = driver.get_calls();
        let g_calls: Vec<_> = calls.iter().filter(|c| c.0 == "gesture").collect();
        assert_eq!(g_calls.len(), 1);
        assert_eq!(
            g_calls[0].1.len(),
            2,
            "rotate SHALL drive exactly two fingers"
        );
    }

    // ── 22. rotate point count scales with the angle ──────────────────

    #[tokio::test]
    async fn rotate_point_count_floor_for_small_angle() {
        // For a small angle (<30deg), the per-finger step count floors at 3,
        // yielding steps+1 = 4 points per finger.
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("rotate");
        step.rotation = Some(5.0);

        handle_rotate_gesture(&step, &driver)
            .await
            .expect("rotate SHALL succeed");

        let calls = driver.get_calls();
        let g_calls: Vec<_> = calls.iter().filter(|c| c.0 == "gesture").collect();
        assert_eq!(g_calls.len(), 1);
        // mock formats each finger as "<n>pts@<dur>ms"; steps floors at 3 -> 4 pts.
        assert!(
            g_calls[0].1[0].starts_with("4pts@"),
            "small angle SHALL floor at 4 points per finger, got: {}",
            g_calls[0].1[0]
        );
    }

    // ── 23. gesture with empty fingers returns error ──────────────────

    #[tokio::test]
    async fn gesture_with_no_fingers_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("gesture"); // fingers empty by default

        let result = handle_gesture(&step, &driver).await;
        assert!(result.is_err(), "gesture with no fingers SHALL error");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("at least one finger"),
            "error SHALL mention the finger requirement, got: {err_msg}"
        );
    }

    // ── 24. gesture with a finger of <2 points returns error ──────────

    #[tokio::test]
    async fn gesture_with_short_finger_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("gesture");
        step.fingers = vec![golem_parser::Finger {
            points: vec![golem_parser::SelectorGroup {
                x: Some(golem_parser::CoordValue::Pixels(10)),
                y: Some(golem_parser::CoordValue::Pixels(10)),
                ..Default::default()
            }],
        }];

        let result = handle_gesture(&step, &driver).await;
        assert!(result.is_err(), "single-point finger SHALL error");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("at least 2 points"),
            "error SHALL mention the 2-point minimum, got: {err_msg}"
        );
    }

    // ── 25. gesture resolves coordinate points and drives the driver ──

    #[tokio::test]
    async fn gesture_with_coordinate_points_drives_driver() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("gesture");
        step.duration = Some(500);
        step.fingers = vec![golem_parser::Finger {
            points: vec![
                golem_parser::SelectorGroup {
                    x: Some(golem_parser::CoordValue::Pixels(10)),
                    y: Some(golem_parser::CoordValue::Pixels(20)),
                    ..Default::default()
                },
                golem_parser::SelectorGroup {
                    x: Some(golem_parser::CoordValue::Pixels(30)),
                    y: Some(golem_parser::CoordValue::Pixels(40)),
                    ..Default::default()
                },
            ],
        }];

        handle_gesture(&step, &driver)
            .await
            .expect("gesture SHALL succeed");

        let calls = driver.get_calls();
        let g_calls: Vec<_> = calls.iter().filter(|c| c.0 == "gesture").collect();
        assert_eq!(g_calls.len(), 1);
        assert_eq!(
            g_calls[0].1,
            vec!["2pts@500ms"],
            "gesture SHALL forward the 2 points with the requested duration"
        );
    }

    // ── 26. gesture point with no element or coords falls back to center ──

    #[tokio::test]
    async fn gesture_point_without_element_or_coords_uses_viewport_center() {
        // A point with no selector and no x/y resolves to viewport center;
        // the resolution still succeeds (no error).
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("gesture");
        step.fingers = vec![golem_parser::Finger {
            points: vec![
                golem_parser::SelectorGroup::default(),
                golem_parser::SelectorGroup {
                    x: Some(golem_parser::CoordValue::Pixels(5)),
                    y: Some(golem_parser::CoordValue::Pixels(5)),
                    ..Default::default()
                },
            ],
        }];

        handle_gesture(&step, &driver)
            .await
            .expect("gesture with center-fallback point SHALL succeed");

        let calls = driver.get_calls();
        let g_calls: Vec<_> = calls.iter().filter(|c| c.0 == "gesture").collect();
        assert_eq!(g_calls.len(), 1);
        // default duration is 300ms
        assert_eq!(g_calls[0].1, vec!["2pts@300ms"]);
    }

    // ── 27. swipe with a 3+ point coordinate path uses a continuous gesture ──

    #[tokio::test]
    async fn swipe_with_three_points_uses_gesture_not_swipe_coords() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.duration = Some(250);
        step.start = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(10)),
            y: Some(golem_parser::CoordValue::Pixels(10)),
            ..Default::default()
        });
        step.points = vec![golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(100)),
            y: Some(golem_parser::CoordValue::Pixels(100)),
            ..Default::default()
        }];
        step.end = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(200)),
            y: Some(golem_parser::CoordValue::Pixels(200)),
            ..Default::default()
        });

        let ctx = test_ctx(Path::new("."));
        handle_swipe(&step, &driver, &ctx)
            .await
            .expect("3-point swipe SHALL succeed");

        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
        let g_calls: Vec<_> = calls.iter().filter(|c| c.0 == "gesture").collect();
        assert!(
            swipe_calls.is_empty(),
            "3+ point swipe SHALL NOT use the 2-point swipe_coords path"
        );
        assert_eq!(g_calls.len(), 1, "3+ point swipe SHALL emit a continuous gesture");
        assert_eq!(g_calls[0].1, vec!["3pts@250ms"]);
    }

    // ── 28. swipe with two coordinate points uses swipe_coords ────────

    #[tokio::test]
    async fn swipe_with_two_coord_points_uses_swipe_coords() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.start = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(10)),
            y: Some(golem_parser::CoordValue::Pixels(20)),
            ..Default::default()
        });
        step.end = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(30)),
            y: Some(golem_parser::CoordValue::Pixels(40)),
            ..Default::default()
        });

        let ctx = test_ctx(Path::new("."));
        handle_swipe(&step, &driver, &ctx)
            .await
            .expect("2-point swipe SHALL succeed");

        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(
            swipe_calls[0].1,
            vec!["10", "20", "30", "40"],
            "the two explicit points SHALL be forwarded verbatim"
        );
    }

    // ── 29. swipe with one point + direction computes the second point ──

    #[tokio::test]
    async fn swipe_with_one_point_plus_direction_computes_end() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.start = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(100)),
            y: Some(golem_parser::CoordValue::Pixels(400)),
            ..Default::default()
        });
        step.params.insert(
            "direction".to_string(),
            toml::Value::String("up".to_string()),
        );

        let ctx = test_ctx(Path::new("."));
        handle_swipe(&step, &driver, &ctx)
            .await
            .expect("one-point + direction swipe SHALL succeed");

        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), 1);
        // dist = vp.height * 2 / 5 = 812 * 2 / 5 = 324; up => y - dist
        assert_eq!(
            swipe_calls[0].1,
            vec!["100", "400", "100", "76"],
            "up direction SHALL move the end point upward by 2/5 of the viewport"
        );
    }

    // ── 30. swipe with one point but no direction returns error ───────

    #[tokio::test]
    async fn swipe_with_one_point_no_direction_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.start = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(100)),
            y: Some(golem_parser::CoordValue::Pixels(400)),
            ..Default::default()
        });

        let ctx = test_ctx(Path::new("."));
        let result = handle_swipe(&step, &driver, &ctx).await;
        assert!(result.is_err(), "one point without direction SHALL error");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("one point requires direction"),
            "error SHALL mention the direction requirement, got: {err_msg}"
        );
    }

    // ── 31. swipe with an element start point uses its center ─────────

    #[tokio::test]
    async fn swipe_from_element_uses_element_center() {
        let root = root_with_button("Card");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("swipe");
        step.start = Some(golem_parser::SelectorGroup {
            text: Some("Card".to_string()),
            ..Default::default()
        });
        step.end = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(0)),
            y: Some(golem_parser::CoordValue::Pixels(0)),
            ..Default::default()
        });

        let ctx = test_ctx(Path::new("."));
        handle_swipe(&step, &driver, &ctx)
            .await
            .expect("element-anchored swipe SHALL succeed");

        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), 1);
        // Button center = (150, 222); end = (0, 0)
        assert_eq!(swipe_calls[0].1, vec!["150", "222", "0", "0"]);
    }

    // ── 32. nudge_into_view swipes up when container overflows the bottom ──

    #[tokio::test]
    async fn nudge_into_view_swipes_up_on_bottom_overflow() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let vp = golem_element::Viewport::new(375, 812);
        // Container starts well below the top and extends past the bottom.
        let bounds = Bounds::new(0, 400, 375, 600);

        nudge_into_view(&driver, &bounds, &vp).await;

        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe_coords").collect();
        assert_eq!(
            swipe_calls.len(),
            1,
            "bottom overflow SHALL trigger exactly one swipe to reveal more"
        );
        // Swipe is upward: from_y > to_y
        let from_y: i32 = swipe_calls[0].1[1].parse().expect("from_y int");
        let to_y: i32 = swipe_calls[0].1[3].parse().expect("to_y int");
        assert!(from_y > to_y, "bottom overflow SHALL swipe upward (from_y > to_y)");
    }

    // ── 33. nudge_into_view does nothing for a fully-visible container ──

    #[tokio::test]
    async fn nudge_into_view_noop_when_fully_visible() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let vp = golem_element::Viewport::new(375, 812);
        // Fully on-screen, small container — neither overflow branch fires.
        let bounds = Bounds::new(20, 100, 300, 200);

        nudge_into_view(&driver, &bounds, &vp).await;

        let calls = driver.get_calls();
        assert!(
            calls.is_empty(),
            "a fully-visible container SHALL trigger no driver calls, got: {calls:?}"
        );
    }
}
