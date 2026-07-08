use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;

use crate::resolution::build_selector;

/// Resolve a coordinate from a TOML param value: integer pixels or "N%" string.
pub(crate) fn resolve_param_coord(val: Option<&toml::Value>, viewport_size: i32) -> i32 {
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
pub(crate) async fn resolve_gesture_center(
    step: &Step,
    driver: &dyn PlatformDriver,
) -> Result<(i32, i32)> {
    let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
    let mut vp = golem_element::Viewport::from_root(&root);
    if meta.keyboard_height > 0 {
        vp.height -= meta.keyboard_height;
    }

    let has_selector =
        step.on_text.is_some() || step.on.is_some() || step.on_accessibility_label.is_some();

    if has_selector {
        let sel = build_selector(step);
        let visible = golem_element::filter_viewport(&root, &vp);
        let results = golem_element::selector::find_elements(&visible, &sel);
        if let Some(first) = results.first() {
            let eb = first.element.effective_bounds();
            Ok((eb.center_x(), eb.center_y()))
        } else {
            crate::fail_code!(
                golem_events::FailureCode::FlowElementNotFound,
                "No element found matching selector"
            );
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
    let scale = step.scale.ok_or_else(|| {
        golem_events::coded(
            golem_events::FailureCode::ParseMissingParam,
            anyhow::anyhow!("pinch requires 'scale' parameter"),
        )
    })?;
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
    let degrees = step.rotation.ok_or_else(|| {
        golem_events::coded(
            golem_events::FailureCode::ParseMissingParam,
            anyhow::anyhow!("rotate requires 'rotation' parameter"),
        )
    })?;
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

    driver
        .gesture(vec![
            golem_driver::GestureFinger {
                points: finger1,
                duration_ms,
            },
            golem_driver::GestureFinger {
                points: finger2,
                duration_ms,
            },
        ])
        .await?;
    Ok(())
}

/// Arbitrary multi-touch gesture with explicit finger paths.
///
/// Each finger in step.fingers has `points` (Vec<SelectorGroup>) resolved to coordinates.
pub(crate) async fn handle_gesture(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    if step.fingers.is_empty() {
        crate::fail_code!(
            golem_events::FailureCode::ParseMissingParam,
            "gesture requires at least one finger in 'fingers' array"
        );
    }

    let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
    let mut vp = golem_element::Viewport::from_root(&root);
    if meta.keyboard_height > 0 {
        vp.height -= meta.keyboard_height;
    }

    let duration = step.duration.unwrap_or(300);

    let mut gesture_fingers = Vec::new();
    for finger in &step.fingers {
        if finger.points.len() < 2 {
            crate::fail_code!(
                golem_events::FailureCode::ParseMissingParam,
                "each finger needs at least 2 points"
            );
        }
        let mut points = Vec::new();
        for group in &finger.points {
            let has_element = group.text.is_some()
                || group.accessibility_label.is_some()
                || group.below.is_some()
                || group.above.is_some();

            if has_element {
                let sel = crate::resolution::build_selector_from_group(group);
                let visible = golem_element::filter_viewport(&root, &vp);
                let results = golem_element::selector::find_elements(&visible, &sel);
                if let Some(first) = results.first() {
                    let base_x = first.element.effective_bounds().center_x();
                    let base_y = first.element.effective_bounds().center_y();
                    let w = first.element.effective_bounds().width;
                    let h = first.element.effective_bounds().height;
                    let x = group
                        .x
                        .as_ref()
                        .map(|xv| {
                            crate::resolution::resolve_coord_public(
                                xv,
                                vp.width,
                                Some(base_x),
                                Some(w),
                                Some(first.element.effective_bounds().x),
                            )
                        })
                        .unwrap_or(base_x);
                    let y = group
                        .y
                        .as_ref()
                        .map(|yv| {
                            crate::resolution::resolve_coord_public(
                                yv,
                                vp.height,
                                Some(base_y),
                                Some(h),
                                Some(first.element.effective_bounds().y),
                            )
                        })
                        .unwrap_or(base_y);
                    points.push((x, y));
                    continue;
                }
            }

            let x = group
                .x
                .as_ref()
                .map(|xv| crate::resolution::resolve_coord_public(xv, vp.width, None, None, None))
                .unwrap_or(vp.width / 2);
            let y = group
                .y
                .as_ref()
                .map(|yv| crate::resolution::resolve_coord_public(yv, vp.height, None, None, None))
                .unwrap_or(vp.height / 2);
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
