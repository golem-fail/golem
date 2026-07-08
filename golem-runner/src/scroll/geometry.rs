use golem_driver::Direction;
use golem_element::{Element, Viewport};

/// Maximum stall attempts (identical hierarchy) before reversing direction.
/// Scrolling down gets more retries because dynamic content typically loads
/// at the bottom (infinite scroll, lazy loading).
const STALL_RETRIES_DOWN: u32 = 3;
const STALL_RETRIES_UP: u32 = 1;
const STALL_RETRIES_DEFAULT: u32 = 2;

/// Swipe distance, as a percentage of viewport/safe-viewport dimension,
/// for the "long swipe" strategies (trailing-edge and near-far-edge
/// starts) — covers ground fast.
const SWIPE_PCT_LONG: u32 = 55;
/// Swipe distance, as a percentage of viewport/safe-viewport dimension,
/// for the "medium/short swipe" strategies (center and side-edge
/// starts) — shorter stride avoids overshooting past small targets.
const SWIPE_PCT_SHORT: u32 = 40;

/// Minimum clear room (px) beside/above/below an absorbing element for
/// `pick_outside_absorber` to route a swipe start point through that
/// side instead of falling through to the next preset strategy.
const ABSORBER_MIN_ROOM_PX: i32 = 80;

/// Inner clamp margin (px) from a container's visible edge for
/// `container_swipe_coords`, so the gesture never grazes the container
/// boundary.
const CONTAINER_EDGE_CLAMP_PX: i32 = 5;

// ── Fingerprinting ──────────────────────────────────────────────────

/// Full hierarchy fingerprint: includes all elements with bounds.
pub(crate) fn hierarchy_fingerprint(root: &Element) -> String {
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
pub(crate) fn horizon_fingerprint(root: &Element, viewport: &Viewport) -> String {
    let strip_height = viewport.height / 8; // top/bottom 12.5%
    let top_strip_bottom = viewport.y + strip_height;
    let bottom_strip_top = viewport.y + viewport.height - strip_height;
    let mut buf = String::new();
    build_horizon_fingerprint(
        root,
        &mut buf,
        viewport.y,
        top_strip_bottom,
        bottom_strip_top,
        viewport.y + viewport.height,
    );
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

pub(crate) fn reverse_direction(dir: Direction) -> Direction {
    match dir {
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
    }
}

pub(crate) fn stall_retries_for(direction: Direction) -> u32 {
    match direction {
        Direction::Down => STALL_RETRIES_DOWN,
        Direction::Up => STALL_RETRIES_UP,
        _ => STALL_RETRIES_DEFAULT,
    }
}

// ── Swipe strategies ────────────────────────────────────────────────

/// A swipe strategy: a finger start position and swipe distance percentage.
pub(crate) struct Strategy {
    pub(crate) start: (i32, i32),
    pub(crate) pct: u32,
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
pub(crate) fn swipe_strategies(viewport: &Viewport, direction: Direction) -> Vec<Strategy> {
    let cx = viewport.width / 2;
    match direction {
        Direction::Down => vec![
            Strategy {
                start: (cx, viewport.height * 65 / 100),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (cx, viewport.height * 90 / 100),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (cx, viewport.height * 50 / 100),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 85 / 100, viewport.height * 65 / 100),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 15 / 100, viewport.height * 65 / 100),
                pct: SWIPE_PCT_SHORT,
            },
        ],
        Direction::Up => vec![
            Strategy {
                start: (cx, viewport.height * 35 / 100),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (cx, viewport.height * 10 / 100),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (cx, viewport.height * 50 / 100),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 85 / 100, viewport.height * 35 / 100),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 15 / 100, viewport.height * 35 / 100),
                pct: SWIPE_PCT_SHORT,
            },
        ],
        Direction::Left => vec![
            Strategy {
                start: (viewport.width * 35 / 100, viewport.height / 2),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (viewport.width * 10 / 100, viewport.height / 2),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (viewport.width * 50 / 100, viewport.height / 2),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 35 / 100, viewport.height * 85 / 100),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 35 / 100, viewport.height * 15 / 100),
                pct: SWIPE_PCT_SHORT,
            },
        ],
        Direction::Right => vec![
            Strategy {
                start: (viewport.width * 65 / 100, viewport.height / 2),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (viewport.width * 90 / 100, viewport.height / 2),
                pct: SWIPE_PCT_LONG,
            },
            Strategy {
                start: (viewport.width * 50 / 100, viewport.height / 2),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 65 / 100, viewport.height * 85 / 100),
                pct: SWIPE_PCT_SHORT,
            },
            Strategy {
                start: (viewport.width * 65 / 100, viewport.height * 15 / 100),
                pct: SWIPE_PCT_SHORT,
            },
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
pub fn make_safe_viewport(vp: &Viewport, meta: &golem_driver::common::HierarchyMeta) -> Viewport {
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
        if !inside {
            return;
        }
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
pub(crate) fn find_absorbing_bounds(
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
        .filter(|b| {
            !(b.x <= safe_vp.x
                && b.x + b.width >= svp_right
                && b.y <= safe_vp.y
                && b.y + b.height >= svp_bottom)
        })
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
pub(crate) fn pick_outside_absorber(
    absorber: golem_element::Bounds,
    direction: Direction,
    safe_vp: &Viewport,
) -> Option<(i32, i32)> {
    let margin = 24;
    let above_abs = (absorber.y - safe_vp.y).saturating_sub(margin);
    let below_abs = (safe_vp.y + safe_vp.height)
        .saturating_sub(absorber.y + absorber.height)
        .saturating_sub(margin);
    let left_abs = (absorber.x - safe_vp.x).saturating_sub(margin);
    let right_abs = (safe_vp.x + safe_vp.width)
        .saturating_sub(absorber.x + absorber.width)
        .saturating_sub(margin);

    let cx = safe_vp.x + safe_vp.width / 2;
    let cy = safe_vp.y + safe_vp.height / 2;

    match direction {
        Direction::Down | Direction::Up => {
            // Prefer side strip with the most room. Pick the cross-axis (x)
            // at the side's midline.
            if left_abs >= ABSORBER_MIN_ROOM_PX && left_abs >= right_abs {
                let x = safe_vp.x + left_abs / 2;
                Some((x, cy))
            } else if right_abs >= ABSORBER_MIN_ROOM_PX {
                let x = absorber.x + absorber.width + right_abs / 2;
                Some((x, cy))
            } else if direction == Direction::Down && above_abs >= ABSORBER_MIN_ROOM_PX {
                // Swipe Down means the finger moves UP from the start; for
                // that we need start ABOVE the absorber so the upward
                // motion stays clear.
                let y = safe_vp.y + above_abs / 2;
                Some((cx, y))
            } else if direction == Direction::Up && below_abs >= ABSORBER_MIN_ROOM_PX {
                let y = absorber.y + absorber.height + below_abs / 2;
                Some((cx, y))
            } else {
                None
            }
        }
        Direction::Left | Direction::Right => {
            if above_abs >= ABSORBER_MIN_ROOM_PX && above_abs >= below_abs {
                Some((cx, safe_vp.y + above_abs / 2))
            } else if below_abs >= ABSORBER_MIN_ROOM_PX {
                Some((cx, absorber.y + absorber.height + below_abs / 2))
            } else if direction == Direction::Left && right_abs >= ABSORBER_MIN_ROOM_PX {
                Some((absorber.x + absorber.width + right_abs / 2, cy))
            } else if direction == Direction::Right && left_abs >= ABSORBER_MIN_ROOM_PX {
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
pub(crate) fn container_swipe_start(
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
            (
                vis_left + (vis_right - vis_left) * 30 / 100,
                (vis_top + vis_bot) / 2,
            )
        }
        Direction::Right => {
            let vis_left = cb.x.max(0);
            let vis_right = (cb.x + cb.width).min(viewport.width);
            (
                vis_left + (vis_right - vis_left) * 70 / 100,
                (vis_top + vis_bot) / 2,
            )
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
pub(crate) fn container_swipe_coords(
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
    let clamp_x = |v: i32| {
        v.max(vis_left + CONTAINER_EDGE_CLAMP_PX)
            .min(vis_right - CONTAINER_EDGE_CLAMP_PX)
    };
    let clamp_y = |v: i32| {
        v.max(vis_top + CONTAINER_EDGE_CLAMP_PX)
            .min(vis_bot - CONTAINER_EDGE_CLAMP_PX)
    };
    let (fx, fy, tx, ty) = match direction {
        Direction::Down => (start.0, start.1, start.0, start.1 - dy),
        Direction::Up => (start.0, start.1, start.0, start.1 + dy),
        Direction::Left => (start.0, start.1, start.0 + dx, start.1),
        Direction::Right => (start.0, start.1, start.0 - dx, start.1),
    };
    (clamp_x(fx), clamp_y(fy), clamp_x(tx), clamp_y(ty))
}
