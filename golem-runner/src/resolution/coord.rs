use golem_element::Viewport;

/// Minimum visible area (px) for an element to be tappable.
const TAP_MARGIN: i32 = 5;

/// Resolve a CoordValue to an absolute pixel position.
///
/// - Standalone (no element): pixels are absolute, percentages are of viewport.
/// - With element: pixels are offset from element center, percentages are of element dimensions.
pub fn resolve_coord_public(
    val: &golem_parser::CoordValue,
    viewport_size: i32,
    element_pos: Option<i32>,
    element_size: Option<i32>,
    element_origin: Option<i32>,
) -> i32 {
    resolve_coord(
        val,
        viewport_size,
        element_pos,
        element_size,
        element_origin,
    )
}

fn resolve_coord(
    val: &golem_parser::CoordValue,
    viewport_size: i32,
    element_pos: Option<i32>,     // element center position
    element_size: Option<i32>,    // element width or height
    _element_origin: Option<i32>, // element x or y
) -> i32 {
    match val {
        golem_parser::CoordValue::Pixels(px) => {
            if let Some(center) = element_pos {
                // Offset from element center
                center + px
            } else {
                // Absolute screen coordinate
                *px
            }
        }
        golem_parser::CoordValue::Percent(pct_str) => {
            let pct: f32 = pct_str.trim_end_matches('%').parse().unwrap_or(0.0) / 100.0;
            if let (Some(center), Some(size)) = (element_pos, element_size) {
                // Percentage of element dimensions from element center
                // 0% = center, 50% = right/bottom edge, -50% = left/top edge
                center + (size as f32 * pct) as i32
            } else {
                // Percentage of viewport
                (viewport_size as f32 * pct) as i32
            }
        }
    }
}

/// Apply x/y coordinate adjustments from the step's selector to tap coordinates.
pub(crate) fn apply_coord_adjustments(
    step: &golem_parser::Step,
    base_x: i32,
    base_y: i32,
    viewport: &Viewport,
    element_bounds: Option<&golem_element::Bounds>,
) -> (i32, i32) {
    let group = step.on.as_ref();
    let x_val = group.and_then(|g| g.x.as_ref());
    let y_val = group.and_then(|g| g.y.as_ref());

    let (elem_cx, elem_cy, elem_w, elem_h, elem_x, elem_y) = if let Some(b) = element_bounds {
        (
            Some(b.center_x()),
            Some(b.center_y()),
            Some(b.width),
            Some(b.height),
            Some(b.x),
            Some(b.y),
        )
    } else {
        (None, None, None, None, None, None)
    };

    let x = if let Some(xv) = x_val {
        resolve_coord(xv, viewport.width, elem_cx, elem_w, elem_x)
    } else {
        base_x
    };

    let y = if let Some(yv) = y_val {
        resolve_coord(yv, viewport.height, elem_cy, elem_h, elem_y)
    } else {
        base_y
    };

    (x, y)
}

/// Compute tap coordinates, preferring the center of the element's portion
/// within the safe zone (below status bar, above nav bar/keyboard).
///
/// If the element is partially in the safe zone, taps the center of that portion.
/// If entirely outside the safe zone, falls back to the element's visible center
/// (taps still work in the danger zone — apps render there).
///
/// Returns None only if the visible portion is too small to tap reliably.
pub(crate) fn safe_tap_coords(
    bounds: &golem_element::Bounds,
    viewport: &Viewport,
    safe_area_top: i32,
    safe_area_bottom: i32,
) -> Option<(i32, i32)> {
    // Intersect element bounds with viewport
    let vis_left = bounds.x.max(0);
    let vis_top = bounds.y.max(0);
    let vis_right = (bounds.x + bounds.width).min(viewport.x + viewport.width);
    let vis_bottom = (bounds.y + bounds.height).min(viewport.y + viewport.height);

    // Check if visible portion is large enough
    if vis_right - vis_left < TAP_MARGIN || vis_bottom - vis_top < TAP_MARGIN {
        return None;
    }

    // Try to tap within the safe zone (preferred)
    let safe_top = vis_top.max(safe_area_top).max(TAP_MARGIN);
    let safe_bottom = vis_bottom
        .min(viewport.y + viewport.height - safe_area_bottom)
        .min(viewport.y + viewport.height - TAP_MARGIN);
    let safe_left = vis_left.max(TAP_MARGIN);
    let safe_right = vis_right.min(viewport.x + viewport.width - TAP_MARGIN);

    if safe_right > safe_left && safe_bottom > safe_top {
        // Element has a portion in the safe zone — tap its center
        Some(((safe_left + safe_right) / 2, (safe_top + safe_bottom) / 2))
    } else {
        // Entirely in the danger zone — fall back to visible center
        Some(((vis_left + vis_right) / 2, (vis_top + vis_bottom) / 2))
    }
}
