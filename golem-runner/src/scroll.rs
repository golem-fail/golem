use std::time::Duration;

use anyhow::Result;
use golem_driver::{Direction, PlatformDriver};
use golem_element::selector::{find_elements, Selector};
use golem_element::{filter_viewport, Element, FindResult, Viewport};
use tokio::time::Instant;

use crate::resolution::wait_for_settle;

/// Maximum stall attempts (identical hierarchy) before reversing direction.
/// Scrolling down gets more retries because dynamic content typically loads
/// at the bottom (infinite scroll, lazy loading).
const STALL_RETRIES_DOWN: u32 = 3;
const STALL_RETRIES_UP: u32 = 1;
const STALL_RETRIES_DEFAULT: u32 = 2;

// ── Fingerprinting ──────────────────────────────────────────────────

/// Full hierarchy fingerprint: includes all elements with bounds.
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

/// Horizon fingerprint: only includes elements whose bounds intersect a thin
/// strip at the top or bottom edge of the viewport. Inner scrollable changes
/// (which happen in the middle of the screen) won't affect this fingerprint.
fn horizon_fingerprint(root: &Element, viewport: &Viewport) -> String {
    let strip_height = viewport.height / 8; // top/bottom 12.5%
    let top_strip_bottom = viewport.y + strip_height;
    let bottom_strip_top = viewport.y + viewport.height - strip_height;
    let mut buf = String::new();
    build_horizon_fingerprint(root, &mut buf, viewport.y, top_strip_bottom, bottom_strip_top, viewport.y + viewport.height);
    buf
}

fn build_horizon_fingerprint(
    element: &Element,
    buf: &mut String,
    top_min: i32,
    top_max: i32,
    bottom_min: i32,
    bottom_max: i32,
) {
    let b = &element.bounds;
    let elem_top = b.y;
    let elem_bottom = b.y + b.height;
    // Element intersects top strip or bottom strip
    let in_top = elem_top < top_max && elem_bottom > top_min;
    let in_bottom = elem_top < bottom_max && elem_bottom > bottom_min;
    if in_top || in_bottom {
        buf.push_str(&element.element_type);
        buf.push(':');
        if let Some(ref text) = element.text {
            buf.push_str(text);
        }
        buf.push_str(&format!("@{},{}", b.x, b.y));
    }
    for child in &element.children {
        build_horizon_fingerprint(child, buf, top_min, top_max, bottom_min, bottom_max);
    }
}

// ── Direction helpers ───────────────────────────────────────────────

fn reverse_direction(dir: Direction) -> Direction {
    match dir {
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
    }
}

fn stall_retries_for(direction: Direction) -> u32 {
    match direction {
        Direction::Down => STALL_RETRIES_DOWN,
        Direction::Up => STALL_RETRIES_UP,
        _ => STALL_RETRIES_DEFAULT,
    }
}

// ── Swipe strategies ────────────────────────────────────────────────

/// A swipe strategy: a finger start position and swipe distance percentage.
struct Strategy {
    start: (i32, i32),
    pct: u32,
}

/// Generate ordered swipe strategies for the given direction.
///
/// Strategies are tried in order when the previous one wastes a swipe
/// (an inner scrollable consumed the gesture instead of the page).
///
/// For Down scroll (finger swipes up):
/// 1. Long swipe from trailing edge (65%) — covers ground fast
/// 2. Long swipe from near bottom (90%) — below most inner scrollables
/// 3. Medium swipe from center (50%) — avoids edge-positioned scrollables
/// 4. Short swipe from right edge — for full-width inner scrollables
/// 5. Short swipe from left edge — for full-width inner scrollables
fn swipe_strategies(viewport: &Viewport, direction: Direction) -> Vec<Strategy> {
    let cx = viewport.width / 2;
    match direction {
        Direction::Down => vec![
            Strategy { start: (cx, viewport.height * 65 / 100), pct: 55 },
            Strategy { start: (cx, viewport.height * 90 / 100), pct: 55 },
            Strategy { start: (cx, viewport.height * 50 / 100), pct: 40 },
            Strategy { start: (viewport.width * 85 / 100, viewport.height * 65 / 100), pct: 40 },
            Strategy { start: (viewport.width * 15 / 100, viewport.height * 65 / 100), pct: 40 },
        ],
        Direction::Up => vec![
            Strategy { start: (cx, viewport.height * 35 / 100), pct: 55 },
            Strategy { start: (cx, viewport.height * 10 / 100), pct: 55 },
            Strategy { start: (cx, viewport.height * 50 / 100), pct: 40 },
            Strategy { start: (viewport.width * 85 / 100, viewport.height * 35 / 100), pct: 40 },
            Strategy { start: (viewport.width * 15 / 100, viewport.height * 35 / 100), pct: 40 },
        ],
        Direction::Left => vec![
            Strategy { start: (viewport.width * 35 / 100, viewport.height / 2), pct: 55 },
            Strategy { start: (viewport.width * 10 / 100, viewport.height / 2), pct: 55 },
            Strategy { start: (viewport.width * 50 / 100, viewport.height / 2), pct: 40 },
            Strategy { start: (viewport.width * 35 / 100, viewport.height * 85 / 100), pct: 40 },
            Strategy { start: (viewport.width * 35 / 100, viewport.height * 15 / 100), pct: 40 },
        ],
        Direction::Right => vec![
            Strategy { start: (viewport.width * 65 / 100, viewport.height / 2), pct: 55 },
            Strategy { start: (viewport.width * 90 / 100, viewport.height / 2), pct: 55 },
            Strategy { start: (viewport.width * 50 / 100, viewport.height / 2), pct: 40 },
            Strategy { start: (viewport.width * 65 / 100, viewport.height * 85 / 100), pct: 40 },
            Strategy { start: (viewport.width * 65 / 100, viewport.height * 15 / 100), pct: 40 },
        ],
    }
}

// ── Swipe coordinate computation ────────────────────────────────────

/// Compute swipe start/end coordinates within a safe viewport.
///
/// `safe_vp` is the rectangle returned by [`make_safe_viewport`] —
/// already accounts for system safe-area insets, keyboard, and any
/// edge-abutting display cutouts. Swipe distance is `pct`% of the
/// safe viewport's dimension on the swipe axis; the start/end points
/// are clamped to a 10% inner margin so the finger never grazes the
/// safe-area edge (where Android can interpret the gesture as a system
/// pull-down / pull-up).
pub fn swipe_from(
    safe_vp: &Viewport,
    direction: Direction,
    start_x: i32,
    start_y: i32,
    swipe_pct: u32,
) -> (i32, i32, i32, i32) {
    let dy = safe_vp.height * swipe_pct as i32 / 100;
    let dx = safe_vp.width * swipe_pct as i32 / 100;

    let min_x = safe_vp.x + safe_vp.width / 10;
    let max_x = safe_vp.x + safe_vp.width * 9 / 10;
    let min_y = safe_vp.y + safe_vp.height / 10;
    let max_y = safe_vp.y + safe_vp.height * 9 / 10;

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

// ── Safe viewport helper ────────────────────────────────────────────

/// Compute the safe viewport — the rectangle in which it's reasonable
/// to start/end a swipe gesture. Subtracts:
///
/// - `safe_area_top` (status bar / notch margin)
/// - `safe_area_bottom` or `keyboard_height` (whichever is larger)
/// - Display cutouts that abut a viewport edge (camera punch-hole at
///   top, dynamic island, notches). Middle-screen cutouts don't
///   reduce the rectangle — a user can swipe around them.
///
/// Single source of truth for "scroll-safe area" — every swipe helper
/// in the runner derives its bounds from this function so notch /
/// cutout handling stays consistent across the action layer and the
/// scroll engine.
pub fn make_safe_viewport(
    vp: &Viewport,
    meta: &golem_driver::common::HierarchyMeta,
) -> Viewport {
    let mut top = vp.y + meta.safe_area_top;
    let bottom_inset = meta.safe_area_bottom.max(meta.keyboard_height);
    let mut bottom = vp.y + vp.height - bottom_inset;
    let mut left = vp.x + meta.safe_area_left;
    let mut right = vp.x + vp.width - meta.safe_area_right;

    // Edge tolerance — a cutout whose edge sits within `tol` of the
    // viewport edge counts as edge-abutting. Real-world cutouts may
    // be reported at exact viewport coords or with a 1-2px gap; 30px
    // covers reasonable margin without picking up mid-screen overlays.
    let tol: i32 = 30;
    for c in &meta.cutouts {
        let c_top = c.y;
        let c_bottom = c.y + c.height;
        let c_left = c.x;
        let c_right = c.x + c.width;

        if c_top <= vp.y + tol {
            top = top.max(c_bottom);
        }
        if c_bottom >= vp.y + vp.height - tol {
            bottom = bottom.min(c_top);
        }
        if c_left <= vp.x + tol {
            left = left.max(c_right);
        }
        if c_right >= vp.x + vp.width - tol {
            right = right.min(c_left);
        }
    }

    Viewport {
        x: left,
        y: top,
        width: (right - left).max(1),
        height: (bottom - top).max(1),
    }
}

/// Walk the tree, collecting every element whose absolute bounds contain
/// the point (x, y). Used to find the visual "stack" at a swipe-start
/// position — the deepest element is the visible top, while shallower
/// elements are its ancestors (frame, scrollable, etc).
fn elements_containing_point(root: &Element, x: i32, y: i32) -> Vec<golem_element::Bounds> {
    fn walk(el: &Element, x: i32, y: i32, out: &mut Vec<golem_element::Bounds>) {
        let b = &el.bounds;
        let inside = b.x <= x && x < b.x + b.width && b.y <= y && y < b.y + b.height;
        if !inside { return; }
        out.push(*b);
        for child in &el.children {
            walk(child, x, y, out);
        }
    }
    let mut result = Vec::new();
    walk(root, x, y, &mut result);
    result
}

/// When a swipe at (x, y) produced no scroll movement, the most likely
/// cause is a sibling/widget at that point absorbing the gesture (e.g. a
/// gesture-area pad, a draggable card, an `onpointerdown` handler with
/// `touch-action: none`). Find the largest non-root element under the
/// point that's big enough to plausibly be the absorber — heuristic
/// threshold ≥20% of viewport area, but smaller than the full viewport
/// (excludes root-like wrappers).
fn find_absorbing_bounds(
    root: &Element,
    x: i32,
    y: i32,
    safe_vp: &Viewport,
) -> Option<golem_element::Bounds> {
    let svp_area = safe_vp.width as i64 * safe_vp.height as i64;
    // Min area: ≥20% of safe viewport. Smaller elements are more
    // likely a button or label than a gesture-trap.
    let min_area = svp_area / 5;
    // Max area: ≤120% of safe viewport. Anything bigger is overflowing
    // scrollable content (HTML `<body>` at 998×8734 on a 1080×2400
    // viewport — 3x area). That's the legitimate scroll container,
    // NOT the absorber. Picking it would degenerate
    // `pick_outside_absorber` because the body wraps almost everything;
    // we want the next-largest sub-element that's plausibly something a
    // finger can route around.
    let max_area = svp_area * 6 / 5;
    let svp_right = safe_vp.x + safe_vp.width;
    let svp_bottom = safe_vp.y + safe_vp.height;
    elements_containing_point(root, x, y)
        .into_iter()
        .filter(|b| {
            let area = b.width as i64 * b.height as i64;
            area >= min_area && area <= max_area
        })
        // Exclude elements that cover the entire safe viewport (or
        // more) in every direction — wrappers (FrameLayout/WebView
        // matching the viewport), full-screen overlays. None of these
        // are avoidable "absorbers"; if one of them really does
        // swallow the swipe, that's a UX issue in the app, not
        // something a smarter swipe origin can route around.
        .filter(|b| !(b.x <= safe_vp.x
            && b.x + b.width >= svp_right
            && b.y <= safe_vp.y
            && b.y + b.height >= svp_bottom))
        .max_by_key(|b| b.width as i64 * b.height as i64)
}

/// Given an absorbing element's bounds and a swipe direction, pick a new
/// swipe start point that avoids the absorber while still letting the
/// swipe travel in the requested direction. Returns `None` when no such
/// point exists in the safe viewport (absorber covers the only useful
/// region for this direction).
///
/// Strategy: for Up/Down scrolls, swap to the OTHER side of the absorber
/// vertically if there's room; otherwise try a side-edge with cross-axis
/// at the absorber's center y. For Left/Right, mirror horizontally.
fn pick_outside_absorber(
    absorber: golem_element::Bounds,
    direction: Direction,
    safe_vp: &Viewport,
) -> Option<(i32, i32)> {
    let margin = 24;
    let above_abs = (absorber.y - safe_vp.y).saturating_sub(margin as i32);
    let below_abs = (safe_vp.y + safe_vp.height)
        .saturating_sub(absorber.y + absorber.height)
        .saturating_sub(margin as i32);
    let left_abs = (absorber.x - safe_vp.x).saturating_sub(margin as i32);
    let right_abs = (safe_vp.x + safe_vp.width)
        .saturating_sub(absorber.x + absorber.width)
        .saturating_sub(margin as i32);

    let cx = safe_vp.x + safe_vp.width / 2;
    let cy = safe_vp.y + safe_vp.height / 2;

    match direction {
        Direction::Down | Direction::Up => {
            // Prefer side strip with the most room. Pick the cross-axis (x)
            // at the side's midline.
            if left_abs >= 80 && left_abs >= right_abs {
                let x = safe_vp.x + left_abs / 2;
                Some((x, cy))
            } else if right_abs >= 80 {
                let x = absorber.x + absorber.width + right_abs / 2;
                Some((x, cy))
            } else if direction == Direction::Down && above_abs >= 80 {
                // Swipe Down means the finger moves UP from the start; for
                // that we need start ABOVE the absorber so the upward
                // motion stays clear.
                let y = safe_vp.y + above_abs / 2;
                Some((cx, y))
            } else if direction == Direction::Up && below_abs >= 80 {
                let y = absorber.y + absorber.height + below_abs / 2;
                Some((cx, y))
            } else {
                None
            }
        }
        Direction::Left | Direction::Right => {
            if above_abs >= 80 && above_abs >= below_abs {
                Some((cx, safe_vp.y + above_abs / 2))
            } else if below_abs >= 80 {
                Some((cx, absorber.y + absorber.height + below_abs / 2))
            } else if direction == Direction::Left && right_abs >= 80 {
                Some((absorber.x + absorber.width + right_abs / 2, cy))
            } else if direction == Direction::Right && left_abs >= 80 {
                Some((safe_vp.x + left_abs / 2, cy))
            } else {
                None
            }
        }
    }
}

/// Default swipe start position for a direction-based swipe (no target element).
/// Used by the swipe action when only a direction is specified.
/// Starts the finger at 65% from the trailing edge, center of the cross-axis.
pub fn default_swipe_start(viewport: &Viewport, direction: Direction) -> (i32, i32) {
    let cx = viewport.width / 2;
    let cy = viewport.height / 2;
    match direction {
        Direction::Down => (cx, viewport.height * 65 / 100),
        Direction::Up => (cx, viewport.height * 35 / 100),
        Direction::Left => (viewport.width * 35 / 100, cy),
        Direction::Right => (viewport.width * 65 / 100, cy),
    }
}

// ── Container swipe geometry ─────────────────────────────────────────

/// Compute the swipe START point for an inner-scrollable (`within`)
/// container, clipped to the visible portion of the container bounds.
///
/// The container's bounds may extend beyond the viewport; this clips to
/// the visible intersection and starts the finger near the trailing edge
/// (70%) on the swipe axis, centered on the cross-axis. Pure geometry —
/// the same value is recomputed on every direction reversal.
fn container_swipe_start(
    cb: &golem_element::Bounds,
    viewport: &Viewport,
    direction: Direction,
) -> (i32, i32) {
    let vis_top = cb.y.max(0);
    let vis_bot = (cb.y + cb.height).min(viewport.height);
    let vis_cx = (cb.x.max(0) + (cb.x + cb.width).min(viewport.width)) / 2;
    match direction {
        Direction::Down => (vis_cx, vis_top + (vis_bot - vis_top) * 70 / 100),
        Direction::Up => (vis_cx, vis_top + (vis_bot - vis_top) * 30 / 100),
        Direction::Left => {
            let vis_left = cb.x.max(0);
            let vis_right = (cb.x + cb.width).min(viewport.width);
            (vis_left + (vis_right - vis_left) * 30 / 100, (vis_top + vis_bot) / 2)
        }
        Direction::Right => {
            let vis_left = cb.x.max(0);
            let vis_right = (cb.x + cb.width).min(viewport.width);
            (vis_left + (vis_right - vis_left) * 70 / 100, (vis_top + vis_bot) / 2)
        }
    }
}

/// Compute the full swipe coordinates (from, to) for one container
/// gesture, given the precomputed `start` point.
///
/// Swipe distance is 80% of the visible container height on the vertical
/// axis and 50% of the visible width on the horizontal axis (the moderate
/// horizontal stride avoids overshooting `scroll-snap` carousels). Both
/// the start and end points are clamped 5px inside the visible container
/// so the gesture never grazes the container edge.
fn container_swipe_coords(
    cb: &golem_element::Bounds,
    viewport: &Viewport,
    direction: Direction,
    start: (i32, i32),
) -> (i32, i32, i32, i32) {
    let vis_top = cb.y.max(0);
    let vis_bot = (cb.y + cb.height).min(viewport.height);
    let vis_left = cb.x.max(0);
    let vis_right = (cb.x + cb.width).min(viewport.width);
    let vis_h = vis_bot - vis_top;
    let vis_w = vis_right - vis_left;
    let dy = vis_h * 80 / 100;
    let dx = vis_w * 50 / 100;
    let clamp_x = |v: i32| v.max(vis_left + 5).min(vis_right - 5);
    let clamp_y = |v: i32| v.max(vis_top + 5).min(vis_bot - 5);
    let (fx, fy, tx, ty) = match direction {
        Direction::Down => (start.0, start.1, start.0, start.1 - dy),
        Direction::Up => (start.0, start.1, start.0, start.1 + dy),
        Direction::Left => (start.0, start.1, start.0 + dx, start.1),
        Direction::Right => (start.0, start.1, start.0 - dx, start.1),
    };
    (clamp_x(fx), clamp_y(fy), clamp_x(tx), clamp_y(ty))
}

// ── Main scroll algorithm ───────────────────────────────────────────

/// Scroll through a view to find an element matching the given selector.
///
/// The algorithm uses a strategy-based approach:
/// 1. Check if the element already exists in the current viewport.
/// 2. Try the primary swipe strategy (long swipe from trailing edge).
/// 3. Use two-tier fingerprinting to detect what happened:
///    - Horizon changed → page scrolled, continue with same strategy.
///    - Horizon unchanged + full changed → inner scrollable consumed gesture,
///      try next strategy.
///    - Both unchanged → possible boundary. Allow stall retries (3 for Down,
///      1 for Up) to handle dynamic content loading, then reverse direction.
/// 4. When a strategy succeeds (page scrolls), promote it to primary.
/// 5. Repeat until element found, timeout, or stall (no-progress).
///
/// The action's timeout is the only wall-clock bound. The number of
/// swipe attempts is unbounded by design — long lists complete; broken
/// trees are caught by stall detection.
pub async fn scroll_to_element(
    selector: &Selector,
    driver: &dyn PlatformDriver,
    initial_direction: Direction,
    timeout_ms: Option<u64>,
    container: Option<golem_element::Bounds>,
    emitter: Option<&golem_events::emitter::DeviceEmitter>,
) -> Result<FindResult> {
    // Step 1: Check current viewport before any scrolling.
    let (mut root, meta, _initial_stats) = wait_for_settle(driver).await?;
    let mut viewport = Viewport::from_root(&root);
    if meta.keyboard_height > 0 {
        viewport.height -= meta.keyboard_height;
    }
    let safe_vp = make_safe_viewport(&viewport, &meta);
    let visible = filter_viewport(&root, &safe_vp);
    let results = find_elements(&visible, selector);
    if let Some(found) = results.into_iter().next() {
        return Ok(found);
    }

    let sel_label = selector.text.as_deref()
        .or(selector.accessibility_label.as_deref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if let Some(ref a) = selector.right_of { return format!("right_of:{a:?}"); }
            if let Some(ref a) = selector.below { return format!("below:{a:?}"); }
            if let Some(ref a) = selector.above { return format!("above:{a:?}"); }
            if let Some(ref a) = selector.left_of { return format!("left_of:{a:?}"); }
            "?".to_string()
        });
    if let Some(e) = emitter {
        e.substep(golem_events::SubstepEvent::ScrollStarted {
            selector: sel_label.clone(),
            direction: format!("{initial_direction:?}"),
        });
    }
    let mut scroll_attempt: u32 = 0;

    let mut direction = initial_direction;
    let mut reversed = false;
    let mut prev_full_fingerprint = hierarchy_fingerprint(&root);
    let mut prev_horizon_fingerprint = horizon_fingerprint(&root, &viewport);
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));

    // Strategy state (page-level scrolling only; containers use fixed geometry)
    let mut strategies = swipe_strategies(&viewport, direction);
    let mut strategy_idx: usize = 0;
    let mut stall_count: u32 = 0;
    // Dynamic-start: when a strategy's preset position lands inside an
    // absorbing widget, try one swipe from an inferred safe spot before
    // falling through to the next preset. Resets on strategy switch /
    // direction reversal.
    let mut dynamic_start_tried: bool = false;
    let mut dynamic_start_override: Option<(i32, i32)> = None;

    // Container swipe start position
    let mut container_start = container
        .as_ref()
        .map(|cb| container_swipe_start(cb, &viewport, direction));

    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            crate::fail_code!(
                golem_events::FailureCode::FlowElementOffscreen,
                "Scroll timed out after {}ms ({scroll_attempt} swipes attempted): \
                 text={:?}, id={:?}",
                timeout_ms.unwrap_or(0),
                selector.text,
                selector.accessibility_label,
            );
        }

        // Compute swipe coordinates
        let (fx, fy, tx, ty) = if let Some(ref cb) = container {
            // Inner-scrollable swipe distance — kept moderate on the
            // horizontal axis because `scroll-snap-type: x mandatory`
            // carousels with finite snap stops will glide past the
            // target on a long, momentum-rich swipe (each 80%-of-
            // container swipe was carrying Card 0 directly to Card 9
            // on Pixel 8 Pro). 50% advances ~1 snap point per swipe
            // on common card widths (~200 CSS px in a ~400 CSS px
            // viewport) so the engine can re-check between gestures.
            let start = *container_start.as_ref().expect("container_start set");
            container_swipe_coords(cb, &viewport, direction, start)
        } else {
            let strat = &strategies[strategy_idx];
            let (sx, sy) = dynamic_start_override.unwrap_or(strat.start);
            swipe_from(&safe_vp, direction, sx, sy, strat.pct)
        };

        scroll_attempt += 1;
        crate::resolution::scroll_swipe_bounded(driver, fx, fy, tx, ty).await?;

        // Check result
        let settle_meta;
        let iter_stats;
        (root, settle_meta, iter_stats) = wait_for_settle(driver).await?;
        let mut vp = Viewport::from_root(&root);
        if settle_meta.keyboard_height > 0 { vp.height -= settle_meta.keyboard_height; }
        let safe_vp = make_safe_viewport(&vp, &settle_meta);
        let visible = filter_viewport(&root, &safe_vp);
        let results = find_elements(&visible, selector);
        if let Some(found) = results.into_iter().next() {
            if let Some(e) = emitter {
                e.substep(golem_events::SubstepEvent::ScrollFound {
                    selector: sel_label.clone(),
                    position: golem_events::Point { x: found.tap_x, y: found.tap_y },
                    total_attempts: scroll_attempt,
                });
            }
            return Ok(found);
        }

        // Overshoot guard: the target may have passed through the viewport
        // between this iteration and the previous one. Even with the
        // dwell-before-lift scroll swipe suppressing fling, a large stride
        // can step past a small target that briefly occupied only a few
        // pixels of the viewport mid-frame. The unfiltered `root` carries
        // the target's document-absolute bounds — if those sit beyond the
        // viewport in the current scroll direction, we've overshot, and
        // continuing in the same direction wastes the remaining budget on
        // ground we already covered. Reverse once and let the next iter
        // catch the target on the way back.
        //
        // Container scrolls (`within`) are excluded — the bounds frame of
        // reference for an inner scrollable carousel doesn't map to the
        // outer viewport the same way, and an explicit `within` already
        // narrows the search to a single container.
        if scroll_attempt > 0 && container.is_none() && !reversed {
            let full_results = find_elements(&root, selector);
            if let Some(found) = full_results.into_iter().next() {
                let b = found.element.bounds;
                let passed = match direction {
                    Direction::Down => b.y + b.height < safe_vp.y,
                    Direction::Up => b.y > safe_vp.y + safe_vp.height,
                    Direction::Left => b.x > safe_vp.x + safe_vp.width,
                    Direction::Right => b.x + b.width < safe_vp.x,
                };
                if passed {
                    if let Some(e) = emitter {
                        e.substep(golem_events::SubstepEvent::ScrollDirectionReversed {
                            to_direction: format!("{:?}", reverse_direction(direction)),
                            reason: format!(
                                "overshoot: target at bounds=({},{},{},{}) past viewport in {direction:?}",
                                b.x, b.y, b.width, b.height,
                            ),
                        });
                    }
                    direction = reverse_direction(direction);
                    strategies = swipe_strategies(&viewport, direction);
                    strategy_idx = 0;
                    stall_count = 0;
                    dynamic_start_tried = false;
                    dynamic_start_override = None;
                    reversed = true;
                    continue;
                }
            }
        }

        // Two-tier fingerprint analysis
        let new_full_fingerprint = hierarchy_fingerprint(&root);
        let new_horizon_fingerprint = horizon_fingerprint(&root, &vp);

        if new_horizon_fingerprint != prev_horizon_fingerprint {
            if let Some(e) = emitter {
                e.substep(golem_events::SubstepEvent::ScrollAttempt {
                    attempt: scroll_attempt,
                    direction: format!("{direction:?}"),
                    strategy_index: strategy_idx,
                    container: container.is_some(),
                    from: golem_events::Point { x: fx, y: fy },
                    to: golem_events::Point { x: tx, y: ty },
                    // For a `within` scroll we don't distinguish page vs inner
                    // movement — any horizon change means the container content
                    // advanced.
                    result: if container.is_some() {
                        golem_events::ScrollAttemptResult::ContainerAdvanced
                    } else {
                        golem_events::ScrollAttemptResult::PageScrolled
                    },
                    tree_stats: iter_stats,
                });
            }
            prev_full_fingerprint = new_full_fingerprint;
            prev_horizon_fingerprint = new_horizon_fingerprint;
            stall_count = 0;
            // A swipe that worked invalidates any pinned dynamic-start —
            // the page has moved, so the absorber-at-(fx,fy) inference
            // we made on a previous stall may no longer hold.
            dynamic_start_tried = false;
            dynamic_start_override = None;
            continue;
        }

        if new_full_fingerprint != prev_full_fingerprint {
            prev_full_fingerprint = new_full_fingerprint;
            if container.is_none() && strategy_idx + 1 < strategies.len() {
                strategy_idx += 1;
                if let Some(e) = emitter {
                    e.substep(golem_events::SubstepEvent::ScrollStrategySwitch {
                        to_index: strategy_idx,
                        reason: "inner scrollable consumed gesture".to_string(),
                    });
                }
                continue;
            }
            if let Some(e) = emitter {
                // For a `within` container, the full-tree change means the
                // inner list/carousel advanced — real progress, not a wasted
                // swipe. For a page scroll with no presets left, it's an inner
                // scrollable eating the gesture.
                let result = if container.is_some() {
                    golem_events::ScrollAttemptResult::ContainerAdvanced
                } else {
                    golem_events::ScrollAttemptResult::InnerScrollableDetected
                };
                e.substep(golem_events::SubstepEvent::ScrollAttempt {
                    attempt: scroll_attempt,
                    direction: format!("{direction:?}"),
                    strategy_index: strategy_idx,
                    container: container.is_some(),
                    from: golem_events::Point { x: fx, y: fy },
                    to: golem_events::Point { x: tx, y: ty },
                    result,
                    tree_stats: iter_stats,
                });
            }
            // When a `within` container is set, scrolling INSIDE the
            // inner scrollable is the explicit intent — full_fingerprint
            // changing means the carousel / list advanced, which is
            // real progress. Reset stall_count and try the same
            // strategy again on the next iteration. Without this
            // reset, the engine falls into stall_count++ and within
            // a few iterations decides the container is stuck,
            // reverses direction, and cycles indefinitely.
            if container.is_some() {
                stall_count = 0;
                continue;
            }
        }

        stall_count += 1;
        let max_stalls = stall_retries_for(direction);
        if stall_count <= max_stalls {
            if let Some(e) = emitter {
                e.substep(golem_events::SubstepEvent::ScrollAttempt {
                    attempt: scroll_attempt,
                    direction: format!("{direction:?}"),
                    strategy_index: strategy_idx,
                    container: container.is_some(),
                    from: golem_events::Point { x: fx, y: fy },
                    to: golem_events::Point { x: tx, y: ty },
                    result: golem_events::ScrollAttemptResult::Stall { count: stall_count, max: max_stalls },
                    tree_stats: iter_stats,
                });
            }
            continue;
        }

        // Before falling through to the next preset strategy, try one
        // dynamic-start swipe at a point inferred to avoid whatever
        // absorbed the previous swipe. Cheap (re-uses the just-fetched
        // tree) and frequently sufficient on pages with a large
        // gesture-trapping widget where the preset strategy positions
        // all happen to land inside the same absorber.
        if container.is_none() && !dynamic_start_tried {
            dynamic_start_tried = true;
            if let Some(absorber) = find_absorbing_bounds(&root, fx, fy, &safe_vp) {
                if let Some(new_start) = pick_outside_absorber(absorber, direction, &safe_vp) {
                    dynamic_start_override = Some(new_start);
                    if let Some(e) = emitter {
                        e.substep(golem_events::SubstepEvent::ScrollStrategySwitch {
                            to_index: strategy_idx,
                            reason: format!(
                                "dynamic start ({},{}) — preset landed inside absorber bounds=({},{},{},{})",
                                new_start.0, new_start.1,
                                absorber.x, absorber.y, absorber.width, absorber.height,
                            ),
                        });
                    }
                    // Reset stall count so the dynamic-start gets its
                    // own retry budget; if it also stalls we fall
                    // through to the preset strategy switch below.
                    stall_count = 0;
                    continue;
                }
            }
        }

        // Stall limit reached on this strategy. Try the next strategy
        // before declaring a boundary — strategies 4/5 swipe off the
        // center column, which is what unsticks pages where an
        // interactive element (e.g. a `pointerdown` handler with
        // `touch-action: none`) sits in the centre and swallows every
        // center-column swipe. Only reverse direction once all
        // strategies have failed.
        //
        // Stall budget carries over (`stall_count` not reset) so the
        // remaining strategies each get exactly one last-chance swipe
        // before reversal, instead of a fresh 3-stall budget per
        // strategy. Worst-case: 3 stalls in strategy 1 + 1 in each of
        // strategies 2–5 = 7 swipes before reversing.
        if container.is_none() && strategy_idx + 1 < strategies.len() {
            strategy_idx += 1;
            dynamic_start_tried = false;
            dynamic_start_override = None;
            if let Some(e) = emitter {
                e.substep(golem_events::SubstepEvent::ScrollStrategySwitch {
                    to_index: strategy_idx,
                    reason: "stall budget exhausted on previous strategy".to_string(),
                });
            }
            continue;
        }

        // All strategies exhausted. Reverse direction.
        if reversed {
            if let Some(e) = emitter {
                e.substep(golem_events::SubstepEvent::ScrollDirectionReversed {
                    to_direction: format!("{:?}", reverse_direction(direction)),
                    reason: "boundary hit again, cycling".to_string(),
                });
            }
            strategy_idx = 0;
            stall_count = 0;
            reversed = false;
            direction = reverse_direction(direction);
            strategies = swipe_strategies(&viewport, direction);
            if let Some(ref cb) = container {
                container_start = Some(container_swipe_start(cb, &viewport, direction));
            }
            continue;
        }

        direction = reverse_direction(direction);
        reversed = true;
        stall_count = 0;
        strategy_idx = 0;
        if let Some(e) = emitter {
            e.substep(golem_events::SubstepEvent::ScrollDirectionReversed {
                to_direction: format!("{direction:?}"),
                reason: "boundary reached".to_string(),
            });
        }
        strategies = swipe_strategies(&viewport, direction);
        if let Some(ref cb) = container {
            container_start = Some(container_swipe_start(cb, &viewport, direction));
        }
    }
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
            visible_bounds: None,
            hit_points: vec![],
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
    fn scroll_intent(args: &[String]) -> &'static str {
        let from_y: i32 = args[1].parse().unwrap();
        let to_y: i32 = args[3].parse().unwrap();
        let from_x: i32 = args[0].parse().unwrap();
        let to_x: i32 = args[2].parse().unwrap();
        let dy = to_y - from_y;
        let dx = to_x - from_x;
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

    struct SequenceMockDriver {
        hierarchies: Mutex<Vec<Element>>,
        call_index: AtomicU32,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl SequenceMockDriver {
        /// Create a mock that returns each hierarchy twice (for settle compatibility).
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
        async fn get_hierarchy(&self) -> anyhow::Result<(Element, golem_driver::common::HierarchyMeta)> {
            self.record_call("get_hierarchy", vec![]);
            let hierarchies = self.hierarchies.lock().expect("lock poisoned");
            let idx = self.call_index.fetch_add(1, Ordering::SeqCst) as usize;
            let clamped = idx.min(hierarchies.len().saturating_sub(1));
            Ok((hierarchies[clamped].clone(), golem_driver::common::HierarchyMeta::default()))
        }

        async fn tap(&self, x: i32, y: i32) -> anyhow::Result<()> {
            self.record_call("tap", vec![x.to_string(), y.to_string()]);
            Ok(())
        }

        async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()> {
            self.record_call("long_press", vec![x.to_string(), y.to_string(), duration_ms.to_string()]);
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

        async fn swipe_coords(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> anyhow::Result<()> {
            self.record_call("swipe_coords", vec![from_x.to_string(), from_y.to_string(), to_x.to_string(), to_y.to_string()]);
            Ok(())
        }

        async fn screenshot(&self) -> anyhow::Result<golem_driver::ScreenshotResult> {
            self.record_call("screenshot", vec![]);
            Ok(golem_driver::ScreenshotResult { path: "mock.png".to_string(), data: vec![] })
        }

        async fn hide_keyboard(&self) -> anyhow::Result<()> { Ok(()) }
        async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<Option<String>> {
            self.record_call("launch_app", vec![bundle_id.to_string()]); Ok(None)
        }
        async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()> {
            self.record_call("stop_app", vec![bundle_id.to_string()]); Ok(())
        }
        async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()> {
            self.record_call("clear_app_data", vec![bundle_id.to_string()]); Ok(())
        }
        async fn press_button(&self, button: &str) -> anyhow::Result<()> {
            self.record_call("press_button", vec![button.to_string()]); Ok(())
        }
        async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()> {
            self.record_call("set_dark_mode", vec![enabled.to_string()]); Ok(())
        }
        async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()> {
            self.record_call("set_location", vec![lat.to_string(), lon.to_string()]); Ok(())
        }
        async fn open_url(&self, url: &str) -> anyhow::Result<()> {
            self.record_call("open_url", vec![url.to_string()]); Ok(())
        }
        async fn push_notification(&self, title: &str, body: &str, payload: Option<&str>) -> anyhow::Result<()> {
            let mut args = vec![title.to_string(), body.to_string()];
            if let Some(p) = payload { args.push(p.to_string()); }
            self.record_call("push_notification", args); Ok(())
        }
        async fn add_media(&self, path: &str) -> anyhow::Result<()> {
            self.record_call("add_media", vec![path.to_string()]); Ok(())
        }
        async fn grant_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
            self.record_call("grant_permission", vec![bundle_id.to_string(), permission.to_string()]); Ok(())
        }
        async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
            self.record_call("revoke_permission", vec![bundle_id.to_string(), permission.to_string()]); Ok(())
        }
        async fn start_recording(&self, name: &str) -> anyhow::Result<()> {
            self.record_call("start_recording", vec![name.to_string()]); Ok(())
        }
        async fn stop_recording(&self) -> anyhow::Result<String> {
            self.record_call("stop_recording", vec![]); Ok("mock.mp4".to_string())
        }
        async fn remove_port_forwards(&self) -> anyhow::Result<()> { Ok(()) }
        async fn pinch(&self, _x: i32, _y: i32, _scale: f64, _velocity: f64) -> anyhow::Result<()> { Ok(()) }
        async fn gesture(&self, fingers: Vec<golem_driver::GestureFinger>) -> anyhow::Result<()> {
            // Scroll swipes route through gesture() with a single finger
            // and a (from, to, to) point sequence — record from + to as
            // the canonical swipe so test helpers can reason about
            // direction without caring about the dwell duplicate.
            if fingers.len() == 1 && fingers[0].points.len() >= 2 {
                let pts = &fingers[0].points;
                self.record_call(
                    "gesture_swipe",
                    vec![
                        pts[0].0.to_string(), pts[0].1.to_string(),
                        pts[1].0.to_string(), pts[1].1.to_string(),
                    ],
                );
            } else {
                self.record_call("gesture", vec![format!("{} fingers", fingers.len())]);
            }
            Ok(())
        }
    }

    // ── 1. Element found in initial hierarchy (no scroll needed) ─────

    #[tokio::test]
    async fn element_found_in_initial_hierarchy() {
        let mut root = make_element("View", default_bounds());
        root.children.push(make_element_with_text("Button", "Target", Bounds::new(10, 10, 100, 44)));

        let driver = MockPlatformDriver::new(root);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, None, None, None)
            .await
            .expect("should find element without scrolling");

        assert_eq!(result.element.text.as_deref(), Some("Target"));
        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "gesture_swipe").collect();
        assert!(swipe_calls.is_empty(), "no swipes SHALL occur");
    }

    // ── 2. Element found after one scroll ───────────────────────────

    #[tokio::test]
    async fn element_found_after_one_scroll() {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Label", "Page 1", Bounds::new(0, 0, 200, 40)));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Button", "Target", Bounds::new(10, 100, 100, 44)));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, None, None, None)
            .await
            .expect("should find element after one scroll");

        assert_eq!(result.element.text.as_deref(), Some("Target"));
        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "gesture_swipe").collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Down");
    }

    // ── 3. Timeout error reports the swipe attempt count ────────────

    #[tokio::test]
    async fn timeout_error_reports_swipe_count() {
        let hierarchies: Vec<Element> = (0..25)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text("Label", &format!("Page {i}"), Bounds::new(0, 0, 200, 40)));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Nonexistent");

        // Tight timeout — driver returns ever-changing trees so stall
        // detection won't trigger; only timeout will.
        let result = scroll_to_element(&selector, &driver, Direction::Down, Some(50), None, None).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(err_msg.contains("Scroll timed out"), "got: {err_msg}");
        assert!(
            err_msg.contains("swipes attempted"),
            "timeout error SHALL include the swipe count for diagnostic context: {err_msg}"
        );
    }

    // ── 4. Bounce detection triggers direction reversal ─────────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn bounce_detection_triggers_direction_reversal() {
        let base = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Label", "Static Page", Bounds::new(0, 0, 200, 40)));
            root
        };
        let different = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Label", "Different Page", Bounds::new(0, 0, 200, 40)));
            root
        };

        // Sequence: many identical entries (stall detection), then different
        // after reversal. Need enough for: initial settle(2) + strategies(5) × settle(2)
        // + stall retries(3) × settle(2) = 2 + 10 + 6 = 18, then different after reverse
        let mut seq: Vec<Element> = std::iter::repeat(base.clone()).take(20).collect();
        seq.extend(std::iter::repeat(different.clone()).take(4));

        let driver = SequenceMockDriver::new(seq);
        let selector = sel_with_text("Nonexistent");

        // Test-only timeout: with no element and no cap, scroll would
        // cycle directions forever. 3s is enough for the test to reach
        // the first reversal across all 5 swipe strategies + stall
        // retries before bailing.
        let _ = scroll_to_element(&selector, &driver, Direction::Down, Some(3000), None, None).await;

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "gesture_swipe").collect();
        let directions: Vec<&str> = swipe_calls.iter().map(|c| scroll_intent(&c.1)).collect();
        assert!(
            directions.contains(&"Up"),
            "direction should reverse after stall, got: {directions:?}"
        );
        let first_up = directions.iter().position(|&d| d == "Up").unwrap();
        assert!(
            directions[..first_up].iter().all(|&d| d == "Down"),
            "all swipes before reversal should be Down"
        );
    }

    // ── 5. Element found after direction reversal ───────────────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn element_found_after_direction_reversal() {
        let base = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Label", "Bottom Page", Bounds::new(0, 0, 200, 40)));
            root
        };
        let with_target = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Button", "Target", Bounds::new(10, 100, 100, 44)));
            root
        };

        // Need enough identical for stall + strategies, then target after reversal
        let mut seq: Vec<Element> = std::iter::repeat(base.clone()).take(20).collect();
        seq.push(with_target.clone());

        let driver = SequenceMockDriver::new(seq);
        let selector = sel_with_text("Target");

        // Test-only timeout — scroll loop without it would cycle
        // forever. ~20 swipes needed to exhaust the stall-cycle and
        // reach the target on the post-reversal pass; 6s leaves headroom.
        let result = scroll_to_element(&selector, &driver, Direction::Down, Some(6000), None, None)
            .await
            .expect("should find element after direction reversal");

        assert_eq!(result.element.text.as_deref(), Some("Target"));

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "gesture_swipe").collect();
        let directions: Vec<&str> = swipe_calls.iter().map(|c| scroll_intent(&c.1)).collect();
        assert!(
            directions.contains(&"Up"),
            "should reverse and find target, got: {directions:?}"
        );
    }

    // ── 7. Empty hierarchy returns error ────────────────────────────

    #[tokio::test]
    async fn empty_hierarchy_returns_error() {
        let root = make_element("View", default_bounds());
        let driver = MockPlatformDriver::new(root);
        let selector = sel_with_text("Anything");

        // Tight test-only timeout: with no element ever appearing, scroll
        // would stall-cycle directions until the timeout. 50ms is enough
        // to verify it errors without slowing the test suite.
        let result = scroll_to_element(&selector, &driver, Direction::Down, Some(50), None, None).await;
        assert!(result.is_err());
    }

    // ── 8-11. Direction tests ───────────────────────────────────────

    async fn direction_test(direction: Direction, expected: &str) {
        let hierarchy_1 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Label", "Page A", Bounds::new(0, 0, 200, 40)));
            root
        };
        let hierarchy_2 = {
            let mut root = make_element("View", default_bounds());
            root.children.push(make_element_with_text("Button", "Found", Bounds::new(10, 10, 100, 44)));
            root
        };

        let driver = SequenceMockDriver::new(vec![hierarchy_1, hierarchy_2]);
        let selector = sel_with_text("Found");

        scroll_to_element(&selector, &driver, direction, None, None, None)
            .await
            .expect("should find element");

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "gesture_swipe").collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), expected);
    }

    #[tokio::test]
    async fn scroll_down_direction_correct() { direction_test(Direction::Down, "Down").await; }

    #[tokio::test]
    async fn scroll_up_direction_correct() { direction_test(Direction::Up, "Up").await; }

    #[tokio::test]
    async fn scroll_left_direction_works() { direction_test(Direction::Left, "Left").await; }

    #[tokio::test]
    async fn scroll_right_direction_works() { direction_test(Direction::Right, "Right").await; }

    // ── 14. Horizon fingerprint detects inner scrollable ────────────

    #[tokio::test]
    async fn horizon_fingerprint_detects_inner_scrollable() {
        let vp = Viewport { x: 0, y: 0, width: 375, height: 812 };

        // Page with header at top and footer at bottom (horizon elements)
        // plus a list in the middle (inner scrollable)
        let mut page1 = make_element("View", default_bounds());
        page1.children.push(make_element_with_text("Header", "Title", Bounds::new(0, 0, 375, 50)));
        page1.children.push(make_element_with_text("List", "Item A", Bounds::new(0, 200, 375, 400)));
        page1.children.push(make_element_with_text("Footer", "Bottom", Bounds::new(0, 770, 375, 42)));

        // Same page but inner list scrolled (different middle content, same edges)
        let mut page2 = make_element("View", default_bounds());
        page2.children.push(make_element_with_text("Header", "Title", Bounds::new(0, 0, 375, 50)));
        page2.children.push(make_element_with_text("List", "Item Z", Bounds::new(0, 200, 375, 400)));
        page2.children.push(make_element_with_text("Footer", "Bottom", Bounds::new(0, 770, 375, 42)));

        // Full fingerprints differ (inner content changed)
        assert_ne!(hierarchy_fingerprint(&page1), hierarchy_fingerprint(&page2));
        // Horizon fingerprints match (top/bottom edges unchanged)
        assert_eq!(horizon_fingerprint(&page1, &vp), horizon_fingerprint(&page2, &vp));
    }

    // ── 15. Horizon fingerprint changes when page scrolls ───────────

    #[tokio::test]
    async fn horizon_fingerprint_changes_when_page_scrolls() {
        let vp = Viewport { x: 0, y: 0, width: 375, height: 812 };

        let mut page1 = make_element("View", default_bounds());
        page1.children.push(make_element_with_text("Header", "Title", Bounds::new(0, 0, 375, 50)));

        // After page scroll, header moved up
        let mut page2 = make_element("View", default_bounds());
        page2.children.push(make_element_with_text("Header", "Title", Bounds::new(0, -200, 375, 50)));
        page2.children.push(make_element_with_text("Section", "New Content", Bounds::new(0, 0, 375, 50)));

        assert_ne!(horizon_fingerprint(&page1, &vp), horizon_fingerprint(&page2, &vp));
    }

    // ── make_safe_viewport ─────────────────────────────────────────

    fn meta_with(
        safe_top: i32,
        safe_bottom: i32,
        keyboard: i32,
        cutouts: Vec<golem_driver::common::CutoutRect>,
    ) -> golem_driver::common::HierarchyMeta {
        golem_driver::common::HierarchyMeta {
            safe_area_top: safe_top,
            safe_area_bottom: safe_bottom,
            keyboard_height: keyboard,
            cutouts,
            ..Default::default()
        }
    }

    fn cutout(x: i32, y: i32, w: i32, h: i32) -> golem_driver::common::CutoutRect {
        golem_driver::common::CutoutRect { x, y, width: w, height: h }
    }

    #[test]
    fn safe_viewport_subtracts_safe_area_insets() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let meta = meta_with(120, 80, 0, vec![]);
        let s = make_safe_viewport(&vp, &meta);
        assert_eq!(s.y, 120);
        assert_eq!(s.height, 2400 - 120 - 80);
    }

    #[test]
    fn safe_viewport_keyboard_overrides_safe_bottom() {
        // Keyboard taller than safe-bottom inset wins.
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let meta = meta_with(120, 80, 900, vec![]);
        let s = make_safe_viewport(&vp, &meta);
        assert_eq!(s.height, 2400 - 120 - 900);
    }

    #[test]
    fn safe_viewport_subtracts_top_edge_cutout() {
        // Punch-hole camera at top: x=480, y=0, 100x100.
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let meta = meta_with(40, 0, 0, vec![cutout(480, 0, 100, 100)]);
        let s = make_safe_viewport(&vp, &meta);
        // Cutout extends to y=100; safe_top was 40. Max wins.
        assert_eq!(s.y, 100, "top edge should be max of safe_top and cutout bottom");
    }

    #[test]
    fn safe_viewport_ignores_mid_screen_cutout() {
        // Hypothetical mid-screen cutout (not realistic but tests the
        // edge-tolerance logic — middle cutouts shouldn't shrink vp).
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let meta = meta_with(40, 0, 0, vec![cutout(500, 1000, 80, 80)]);
        let s = make_safe_viewport(&vp, &meta);
        // No edge match — only safe_top applies.
        assert_eq!(s.y, 40);
        assert_eq!(s.height, 2360);
    }

    // ── find_absorbing_bounds ──────────────────────────────────────

    #[test]
    fn absorber_excludes_full_viewport_wrapper() {
        // A FrameLayout matching the viewport exactly should be
        // excluded — it's a wrapper, not an absorber.
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let mut root = make_element("View", Bounds::new(0, 0, 1080, 2400));
        root.children.push(make_element("FrameLayout", Bounds::new(0, 0, 1080, 2400)));
        assert!(find_absorbing_bounds(&root, 500, 1000, &vp).is_none());
    }

    #[test]
    fn absorber_excludes_taller_than_viewport_body() {
        // HTML body taller than viewport (scrollable content) should
        // not be picked as the absorber.
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let mut root = make_element("View", Bounds::new(0, 0, 1080, 2400));
        root.children.push(make_element("body", Bounds::new(0, 0, 1080, 9000)));
        assert!(find_absorbing_bounds(&root, 500, 1000, &vp).is_none());
    }

    #[test]
    fn absorber_excludes_overflowing_body_with_horizontal_margin() {
        // Real case from sweep recover4: HTML body at (42,-3397,998,8734)
        // on Pixel 7a (1080x2400 viewport). Doesn't reach left/right
        // edges but overflows top + bottom dramatically. Area cap
        // should exclude it (8.7M area vs 2.6M viewport area).
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let mut root = make_element("View", Bounds::new(0, 0, 1080, 2400));
        root.children.push(make_element("body", Bounds::new(42, -3397, 998, 8734)));
        assert!(
            find_absorbing_bounds(&root, 540, 2115, &vp).is_none(),
            "overflowing body should not be picked as absorber"
        );
    }

    #[test]
    fn safe_viewport_subtracts_left_right_insets() {
        let vp = Viewport { x: 0, y: 0, width: 1344, height: 2992 };
        let meta = meta_with(187, 96, 0, vec![]);
        // Build a meta with all four sides populated. meta_with only
        // covers top/bottom/kb/cutouts, so construct directly here.
        let mut meta = meta;
        meta.safe_area_left = 90;
        meta.safe_area_right = 90;
        let s = make_safe_viewport(&vp, &meta);
        assert_eq!(s.x, 90);
        assert_eq!(s.width, 1344 - 90 - 90);
        assert_eq!(s.y, 187);
        assert_eq!(s.height, 2992 - 187 - 96);
    }

    #[test]
    fn absorber_picks_largest_sub_viewport_element() {
        // A widget covering 1000×1000 at (40, 800) is the plausible
        // absorber when the point lands inside it.
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let mut root = make_element("View", Bounds::new(0, 0, 1080, 2400));
        let mut wrapper = make_element("FrameLayout", Bounds::new(0, 0, 1080, 2400));
        let widget = make_element("div", Bounds::new(40, 800, 1000, 1000));
        wrapper.children.push(widget);
        root.children.push(wrapper);
        let absorber = find_absorbing_bounds(&root, 500, 1200, &vp)
            .expect("widget should be picked");
        assert_eq!(absorber.x, 40);
        assert_eq!(absorber.y, 800);
        assert_eq!(absorber.width, 1000);
        assert_eq!(absorber.height, 1000);
    }

    // ── pick_outside_absorber ──────────────────────────────────────

    #[test]
    fn pick_outside_absorber_returns_side_when_room() {
        // Absorber spans middle 60% of width — prefer side strips
        // for vertical scrolls.
        let safe_vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let absorber = golem_element::Bounds::new(216, 600, 648, 1200);
        let p = pick_outside_absorber(absorber, Direction::Down, &safe_vp)
            .expect("side strip exists");
        // Should be either left strip (x < 216) or right strip (x > 864).
        assert!(p.0 < 216 || p.0 > 864, "expected side strip, got x={}", p.0);
    }

    #[test]
    fn pick_outside_absorber_falls_back_to_above_when_no_sides() {
        // Absorber spans full width but only bottom half.
        let safe_vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let absorber = golem_element::Bounds::new(0, 1200, 1080, 1200);
        let p = pick_outside_absorber(absorber, Direction::Down, &safe_vp)
            .expect("above strip exists");
        assert!(p.1 < 1200, "expected above strip, got y={}", p.1);
    }

    #[test]
    fn pick_outside_absorber_none_when_full_cover() {
        let safe_vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let absorber = golem_element::Bounds::new(0, 0, 1080, 2400);
        assert!(pick_outside_absorber(absorber, Direction::Down, &safe_vp).is_none());
    }

    // ── reverse_direction ──────────────────────────────────────────

    // 1. Each direction reverses to its opposite and round-trips back.
    #[test]
    fn reverse_direction_maps_each_axis_pair() {
        assert!(matches!(reverse_direction(Direction::Up), Direction::Down), "Up SHALL reverse to Down");
        assert!(matches!(reverse_direction(Direction::Down), Direction::Up), "Down SHALL reverse to Up");
        assert!(matches!(reverse_direction(Direction::Left), Direction::Right), "Left SHALL reverse to Right");
        assert!(matches!(reverse_direction(Direction::Right), Direction::Left), "Right SHALL reverse to Left");
        // Round-trip: reversing twice yields the original.
        for d in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
            assert!(
                std::mem::discriminant(&reverse_direction(reverse_direction(d))) == std::mem::discriminant(&d),
                "double reverse SHALL return the original direction"
            );
        }
    }

    // ── stall_retries_for ──────────────────────────────────────────

    // 2. Down gets the most retries (dynamic content loads at bottom),
    //    Up the least, and the cross-axis directions get the default.
    #[test]
    fn stall_retries_per_direction() {
        // 1. Pin the concrete intended budgets as hand-written literals so a
        //    change to the constant VALUE is caught, not just a swapped arm.
        assert_eq!(stall_retries_for(Direction::Down), 3, "Down SHALL get 3 retries (dynamic content loads at bottom)");
        assert_eq!(stall_retries_for(Direction::Up), 1, "Up SHALL get 1 retry");
        assert_eq!(stall_retries_for(Direction::Left), 2, "Left SHALL get the default 2 retries");
        assert_eq!(stall_retries_for(Direction::Right), 2, "Right SHALL get the default 2 retries");
        // 2. Down SHALL have strictly more retries than Up by design.
        assert!(stall_retries_for(Direction::Down) > stall_retries_for(Direction::Up), "Down budget SHALL exceed Up budget");
    }

    // ── swipe_strategies ───────────────────────────────────────────

    // 3. Every direction yields exactly five ordered strategies, each
    //    with a positive swipe percentage and a start inside the viewport.
    #[test]
    fn swipe_strategies_count_and_bounds() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        for d in [Direction::Down, Direction::Up, Direction::Left, Direction::Right] {
            let strats = swipe_strategies(&vp, d);
            assert_eq!(strats.len(), 5, "each direction SHALL produce 5 strategies");
            for s in &strats {
                assert!(s.pct > 0 && s.pct <= 100, "pct SHALL be in (0,100], got {}", s.pct);
                assert!(s.start.0 >= 0 && s.start.0 <= vp.width, "start x SHALL be within viewport width");
                assert!(s.start.1 >= 0 && s.start.1 <= vp.height, "start y SHALL be within viewport height");
            }
        }
    }

    // 4. The first Down strategy starts in the lower-middle (65%) of the
    //    viewport at the horizontal center — the long primary swipe.
    #[test]
    fn swipe_strategies_down_primary_geometry() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let strats = swipe_strategies(&vp, Direction::Down);
        assert_eq!(strats[0].start, (540, 2400 * 65 / 100), "Down primary SHALL start at center, 65% down");
        assert_eq!(strats[0].pct, 55, "Down primary SHALL be a long (55%) swipe");
    }

    // ── swipe_from ─────────────────────────────────────────────────

    // 5. A Down swipe (finger moves up) keeps x fixed and decreases y by
    //    pct% of the safe-viewport height, both points clamped to the
    //    10%..90% inner margin.
    #[test]
    fn swipe_from_down_moves_finger_up() {
        let safe_vp = Viewport { x: 0, y: 0, width: 1000, height: 1000 };
        let (fx, fy, tx, ty) = swipe_from(&safe_vp, Direction::Down, 500, 700, 50);
        assert_eq!(fx, 500, "x SHALL be unchanged on a vertical swipe");
        assert_eq!(tx, 500, "x SHALL be unchanged on a vertical swipe");
        assert!(ty < fy, "Down (finger up) SHALL produce a smaller end y");
        // dy = 50% of 1000 = 500; raw end = 700 - 500 = 200, within margin.
        assert_eq!(fy, 700);
        assert_eq!(ty, 200);
    }

    // 6. Up/Left/Right move the finger the opposite way to their scroll
    //    intent on the correct axis.
    #[test]
    fn swipe_from_other_directions_axis_and_sign() {
        let safe_vp = Viewport { x: 0, y: 0, width: 1000, height: 1000 };
        // Up: finger moves down (end y larger).
        let (_, fy, _, ty) = swipe_from(&safe_vp, Direction::Up, 500, 300, 40);
        assert!(ty > fy, "Up (finger down) SHALL produce a larger end y");
        // Left: finger moves right (end x larger), y fixed.
        let (fx, fyl, tx, tyl) = swipe_from(&safe_vp, Direction::Left, 300, 500, 40);
        assert!(tx > fx, "Left (finger right) SHALL produce a larger end x");
        assert_eq!(fyl, tyl, "Left SHALL keep y fixed");
        // Right: finger moves left (end x smaller).
        let (fxr, _, txr, _) = swipe_from(&safe_vp, Direction::Right, 700, 500, 40);
        assert!(txr < fxr, "Right (finger left) SHALL produce a smaller end x");
    }

    // 7. Start/end points are clamped to the 10%..90% inner margin so the
    //    finger never grazes the safe-area edge.
    #[test]
    fn swipe_from_clamps_to_inner_margin() {
        let safe_vp = Viewport { x: 0, y: 0, width: 1000, height: 1000 };
        // Start far above the top margin and a large swipe; both points
        // must clamp into [100, 900].
        let (fx, fy, tx, ty) = swipe_from(&safe_vp, Direction::Down, 5, 50, 100);
        for (label, v) in [("fx", fx), ("fy", fy), ("tx", tx), ("ty", ty)] {
            assert!((100..=900).contains(&v), "{label}={v} SHALL clamp into 10%..90% margin");
        }
    }

    // 8. Clamp respects a non-zero viewport origin (x/y offset).
    #[test]
    fn swipe_from_clamp_respects_origin_offset() {
        let safe_vp = Viewport { x: 200, y: 100, width: 1000, height: 1000 };
        // min_x = 200 + 100 = 300, max_x = 200 + 900 = 1100.
        let (fx, _, _, _) = swipe_from(&safe_vp, Direction::Down, 0, 500, 20);
        assert_eq!(fx, 300, "x SHALL clamp to the offset min, not 0");
    }

    // ── default_swipe_start ────────────────────────────────────────

    // 9. Default start sits 65% down for Down, 35% down for Up, both at
    //    horizontal center; Left/Right mirror that on the x axis at center y.
    #[test]
    fn default_swipe_start_per_direction() {
        let vp = Viewport { x: 0, y: 0, width: 1000, height: 1000 };
        assert_eq!(default_swipe_start(&vp, Direction::Down), (500, 650));
        assert_eq!(default_swipe_start(&vp, Direction::Up), (500, 350));
        assert_eq!(default_swipe_start(&vp, Direction::Left), (350, 500));
        assert_eq!(default_swipe_start(&vp, Direction::Right), (650, 500));
    }

    // ── make_safe_viewport — left/right edge cutouts & clamp ───────

    // 10. A cutout abutting the left edge pushes the left bound to its
    //     right edge; one abutting the right edge pulls the right bound in.
    #[test]
    fn safe_viewport_subtracts_side_edge_cutouts() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        // Left cutout 0..60, right cutout 1000..1080.
        let meta = meta_with(0, 0, 0, vec![cutout(0, 1000, 60, 100), cutout(1000, 1000, 80, 100)]);
        let s = make_safe_viewport(&vp, &meta);
        assert_eq!(s.x, 60, "left edge SHALL move to left-cutout's right edge");
        // right bound becomes 1000, width = 1000 - 60.
        assert_eq!(s.width, 1000 - 60, "right edge SHALL pull in to right-cutout's left edge");
    }

    // 11. A bottom-edge cutout pulls the bottom bound up to the cutout top.
    #[test]
    fn safe_viewport_subtracts_bottom_edge_cutout() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        // Cutout at very bottom: y=2350..2400.
        let meta = meta_with(0, 0, 0, vec![cutout(490, 2350, 100, 50)]);
        let s = make_safe_viewport(&vp, &meta);
        assert_eq!(s.height, 2350, "bottom SHALL clamp to bottom-cutout's top");
    }

    // 12. When insets exceed the viewport, width/height never go below 1.
    #[test]
    fn safe_viewport_clamps_to_minimum_one() {
        let vp = Viewport { x: 0, y: 0, width: 100, height: 100 };
        // Top + bottom insets sum to more than the height.
        let meta = meta_with(80, 80, 0, vec![]);
        let s = make_safe_viewport(&vp, &meta);
        assert_eq!(s.height, 1, "height SHALL clamp to a minimum of 1");
        assert!(s.width >= 1, "width SHALL never be below 1");
    }

    // ── find_absorbing_bounds — area floor ─────────────────────────

    // 13. An element smaller than 20% of the safe viewport is below the
    //     absorber floor and is not picked (likely a button/label).
    #[test]
    fn absorber_ignores_too_small_element() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let mut root = make_element("View", Bounds::new(0, 0, 1080, 2400));
        // 100x100 = 10k area, far below 20% of 2.59M.
        root.children.push(make_element("button", Bounds::new(450, 1150, 100, 100)));
        assert!(
            find_absorbing_bounds(&root, 500, 1200, &vp).is_none(),
            "sub-threshold element SHALL NOT be picked as absorber"
        );
    }

    // 14. No element under the point yields None.
    #[test]
    fn absorber_none_when_point_outside_all() {
        let vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        let mut root = make_element("View", Bounds::new(0, 0, 1080, 2400));
        root.children.push(make_element("div", Bounds::new(40, 800, 1000, 1000)));
        // Point (10, 10) is in the root wrapper only (excluded), not the div.
        assert!(find_absorbing_bounds(&root, 10, 10, &vp).is_none());
    }

    // ── pick_outside_absorber — Up / Left / Right ──────────────────

    // 15. Up scroll with a full-width absorber in the top half falls back
    //     to the strip BELOW the absorber (finger needs room below).
    #[test]
    fn pick_outside_absorber_up_falls_back_below() {
        let safe_vp = Viewport { x: 0, y: 0, width: 1080, height: 2400 };
        // Full width, top half — no side room.
        let absorber = golem_element::Bounds::new(0, 0, 1080, 1200);
        let p = pick_outside_absorber(absorber, Direction::Up, &safe_vp)
            .expect("below strip exists");
        assert!(p.1 > 1200, "Up SHALL fall back to a start below the absorber, got y={}", p.1);
    }

    // 16. Left/Right scrolls prefer an above/below strip when the absorber
    //     leaves no vertical room for a same-axis side start.
    #[test]
    fn pick_outside_absorber_horizontal_prefers_above() {
        let safe_vp = Viewport { x: 0, y: 0, width: 2400, height: 1080 };
        // Tall absorber spanning the full height's middle band but leaving
        // an above strip: y=300..1080 leaves 300px above.
        let absorber = golem_element::Bounds::new(600, 300, 1200, 780);
        let p = pick_outside_absorber(absorber, Direction::Left, &safe_vp)
            .expect("above strip exists");
        assert!(p.1 < 300, "horizontal scroll SHALL prefer the above strip, got y={}", p.1);
    }

    // 17. Right scroll with a full-height absorber on the right side falls
    //     back to the strip to its LEFT.
    #[test]
    fn pick_outside_absorber_right_falls_back_left() {
        let safe_vp = Viewport { x: 0, y: 0, width: 2400, height: 1080 };
        // Full height so no above/below room; occupies right portion,
        // leaving a left strip.
        let absorber = golem_element::Bounds::new(1200, 0, 1200, 1080);
        let p = pick_outside_absorber(absorber, Direction::Right, &safe_vp)
            .expect("left strip exists");
        assert!(p.0 < 1200, "Right SHALL fall back to a left-side start, got x={}", p.0);
    }

    // ── horizon_fingerprint — empty edges ──────────────────────────

    // 18. A page with content only in the dead-center (outside both the
    //     top and bottom strips) yields an empty horizon fingerprint, so
    //     two such pages with differing center content compare equal.
    #[test]
    fn horizon_fingerprint_ignores_center_only_content() {
        let vp = Viewport { x: 0, y: 0, width: 375, height: 812 };
        let mut p1 = make_element("View", default_bounds());
        // Center band only: strip height = 812/8 ~= 101; center at ~400.
        p1.children.push(make_element_with_text("Mid", "Alpha", Bounds::new(0, 350, 375, 50)));
        let mut p2 = make_element("View", default_bounds());
        p2.children.push(make_element_with_text("Mid", "Omega", Bounds::new(0, 350, 375, 50)));
        // The root itself spans the full viewport, so it appears in both;
        // center children differ but sit outside both strips.
        assert_eq!(
            horizon_fingerprint(&p1, &vp),
            horizon_fingerprint(&p2, &vp),
            "center-only differences SHALL NOT affect the horizon fingerprint"
        );
    }

    // ── container_swipe_start ──────────────────────────────────────

    // 19. A fully-visible container starts the finger near the trailing
    //     edge on the swipe axis (70% for Down, 30% for Up) at the
    //     container's horizontal center.
    #[test]
    fn container_swipe_start_vertical_geometry() {
        let vp = Viewport { x: 0, y: 0, width: 400, height: 1000 };
        // Container occupies y=200..600 (visible height 400), x=50..350.
        let cb = Bounds::new(50, 200, 300, 400);
        // 1. Down starts at 70% of the visible height from its top edge.
        let down = container_swipe_start(&cb, &vp, Direction::Down);
        assert_eq!(down, (200, 200 + 400 * 70 / 100), "Down SHALL start at center x, 70% down the container");
        // 2. Up starts at 30% from the top so the finger has room to move down.
        let up = container_swipe_start(&cb, &vp, Direction::Up);
        assert_eq!(up, (200, 200 + 400 * 30 / 100), "Up SHALL start at center x, 30% down the container");
    }

    // 20. Horizontal scrolls start near the trailing horizontal edge
    //     (30% for Left, 70% for Right) at the container's vertical center.
    #[test]
    fn container_swipe_start_horizontal_geometry() {
        let vp = Viewport { x: 0, y: 0, width: 400, height: 1000 };
        let cb = Bounds::new(50, 200, 300, 400);
        let cy = (200 + 600) / 2;
        let left = container_swipe_start(&cb, &vp, Direction::Left);
        assert_eq!(left, (50 + 300 * 30 / 100, cy), "Left SHALL start at 30% across, center y");
        let right = container_swipe_start(&cb, &vp, Direction::Right);
        assert_eq!(right, (50 + 300 * 70 / 100, cy), "Right SHALL start at 70% across, center y");
    }

    // 21. A container extending beyond the viewport is clipped to the
    //     visible intersection before the geometry is computed.
    #[test]
    fn container_swipe_start_clips_to_visible() {
        let vp = Viewport { x: 0, y: 0, width: 400, height: 1000 };
        // Container starts above the top edge (y=-300) and runs past the
        // bottom — visible band is y=0..1000.
        let cb = Bounds::new(0, -300, 400, 2000);
        let down = container_swipe_start(&cb, &vp, Direction::Down);
        // vis_top=0, vis_bot=1000 → 70% of 1000 = 700.
        assert_eq!(down.1, 700, "start SHALL use the clipped visible band, not raw bounds");
        assert!(down.1 < vp.height, "start y SHALL stay within the viewport");
    }

    // ── container_swipe_coords ─────────────────────────────────────

    // 22. A Down container swipe keeps x fixed and moves the finger up by
    //     80% of the visible container height; both points clamp 5px
    //     inside the visible band.
    #[test]
    fn container_swipe_coords_down_moves_up_and_clamps() {
        let vp = Viewport { x: 0, y: 0, width: 400, height: 1000 };
        let cb = Bounds::new(0, 100, 400, 400); // visible y=100..500
        let start = container_swipe_start(&cb, &vp, Direction::Down);
        let (fx, fy, tx, ty) = container_swipe_coords(&cb, &vp, Direction::Down, start);
        assert_eq!(fx, tx, "x SHALL stay fixed on a vertical container swipe");
        assert!(ty < fy, "Down (finger up) SHALL produce a smaller end y");
        // Every coordinate SHALL be clamped 5px inside the visible band.
        for (label, v) in [("fy", fy), ("ty", ty)] {
            assert!((105..=495).contains(&v), "{label}={v} SHALL clamp 5px inside the visible container");
        }
    }

    // 23. Left/Right container swipes move along x by 50% of the visible
    //     width (the moderate snap-carousel stride) with y fixed.
    #[test]
    fn container_swipe_coords_horizontal_half_stride() {
        let vp = Viewport { x: 0, y: 0, width: 1000, height: 600 };
        let cb = Bounds::new(0, 0, 1000, 600);
        // Left: finger moves right (end x larger), y fixed.
        let lstart = container_swipe_start(&cb, &vp, Direction::Left);
        let (lfx, lfy, ltx, lty) = container_swipe_coords(&cb, &vp, Direction::Left, lstart);
        assert!(ltx > lfx, "Left (finger right) SHALL produce a larger end x");
        assert_eq!(lfy, lty, "Left SHALL keep y fixed");
        // Right: finger moves left (end x smaller).
        let rstart = container_swipe_start(&cb, &vp, Direction::Right);
        let (rfx, _, rtx, _) = container_swipe_coords(&cb, &vp, Direction::Right, rstart);
        assert!(rtx < rfx, "Right (finger left) SHALL produce a smaller end x");
    }
}
