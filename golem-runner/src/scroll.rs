use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::{Direction, PlatformDriver};
use golem_element::selector::{find_elements, Selector};
use golem_element::{filter_viewport, Element, FindResult, Viewport};
use tokio::time::Instant;

use crate::resolution::wait_for_settle;

/// Default maximum number of scroll attempts before giving up.
pub const DEFAULT_MAX_SCROLLS: u32 = 20;

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

/// Compute swipe coordinates: the swipe starts at `(start_x, start_y)` and
/// travels `swipe_pct`% of the screen in the given direction.
///
/// Clamps all coordinates to 10%-90% of the screen (or safe area insets)
/// to avoid system gesture areas (notification bar, home indicator).
pub fn swipe_from(
    viewport: &Viewport,
    direction: Direction,
    start_x: i32,
    start_y: i32,
    swipe_pct: u32,
) -> (i32, i32, i32, i32) {
    swipe_from_with_insets(viewport, direction, start_x, start_y, swipe_pct, 0, 0)
}

/// Like `swipe_from` but uses safe area insets to avoid system gesture zones.
pub fn swipe_from_with_insets(
    viewport: &Viewport,
    direction: Direction,
    start_x: i32,
    start_y: i32,
    swipe_pct: u32,
    safe_area_top: i32,
    safe_area_bottom: i32,
) -> (i32, i32, i32, i32) {
    let dy = viewport.height * swipe_pct as i32 / 100;
    let dx = viewport.width * swipe_pct as i32 / 100;

    let min_x = viewport.width / 10;
    let max_x = viewport.width * 9 / 10;
    // Add margin above safe area to avoid accidentally triggering system gestures
    // (e.g. Android notification shade) when swiping near the status bar.
    let safe_margin = viewport.height / 20; // 5% margin
    let min_y = (viewport.height / 10).max(safe_area_top + safe_margin);
    let max_y = (viewport.height * 9 / 10).min(viewport.height - safe_area_bottom - safe_margin);

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

fn make_safe_viewport(
    vp: &Viewport,
    meta: &golem_driver::common::HierarchyMeta,
) -> Viewport {
    let mut safe = *vp;
    if meta.safe_area_top > 0 {
        safe.y += meta.safe_area_top;
        safe.height -= meta.safe_area_top;
    }
    if meta.safe_area_bottom > meta.keyboard_height {
        safe.height -= meta.safe_area_bottom - meta.keyboard_height;
    }
    safe
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
/// 5. Repeat until element found, timeout, or max_scrolls exhausted.
pub async fn scroll_to_element(
    selector: &Selector,
    driver: &dyn PlatformDriver,
    initial_direction: Direction,
    max_scrolls: u32,
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
    let mut prev_full_fp = hierarchy_fingerprint(&root);
    let mut prev_horizon_fp = horizon_fingerprint(&root, &viewport);
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));

    // Strategy state (page-level scrolling only; containers use fixed geometry)
    let mut strategies = swipe_strategies(&viewport, direction);
    let mut strategy_idx: usize = 0;
    let mut stall_count: u32 = 0;

    // Container swipe start position
    let mut container_start = container.as_ref().map(|cb| {
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
    });

    #[allow(clippy::explicit_counter_loop)]
    for _ in 0..max_scrolls {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            bail!(
                "Scroll timed out after {}ms: text={:?}, id={:?}",
                timeout_ms.unwrap_or(0),
                selector.text,
                selector.accessibility_label,
            );
        }

        // Compute swipe coordinates
        let (fx, fy, tx, ty) = if let Some(ref cb) = container {
            let start = container_start.as_ref().expect("container_start set");
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
            let strat = &strategies[strategy_idx];
            swipe_from_with_insets(
                &viewport, direction, strat.start.0, strat.start.1, strat.pct,
                meta.safe_area_top, meta.safe_area_bottom.max(meta.keyboard_height),
            )
        };

        scroll_attempt += 1;
        driver.swipe_coords(fx, fy, tx, ty).await?;

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

        // Two-tier fingerprint analysis
        let new_full_fp = hierarchy_fingerprint(&root);
        let new_horizon_fp = horizon_fingerprint(&root, &vp);

        if new_horizon_fp != prev_horizon_fp {
            if let Some(e) = emitter {
                e.substep(golem_events::SubstepEvent::ScrollAttempt {
                    attempt: scroll_attempt,
                    direction: format!("{direction:?}"),
                    strategy_index: strategy_idx,
                    from: golem_events::Point { x: fx, y: fy },
                    to: golem_events::Point { x: tx, y: ty },
                    result: golem_events::ScrollAttemptResult::PageScrolled,
                    tree_stats: iter_stats,
                });
            }
            prev_full_fp = new_full_fp;
            prev_horizon_fp = new_horizon_fp;
            stall_count = 0;
            continue;
        }

        if new_full_fp != prev_full_fp {
            prev_full_fp = new_full_fp;
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
                e.substep(golem_events::SubstepEvent::ScrollAttempt {
                    attempt: scroll_attempt,
                    direction: format!("{direction:?}"),
                    strategy_index: strategy_idx,
                    from: golem_events::Point { x: fx, y: fy },
                    to: golem_events::Point { x: tx, y: ty },
                    result: golem_events::ScrollAttemptResult::InnerScrollableDetected,
                    tree_stats: iter_stats,
                });
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
                    from: golem_events::Point { x: fx, y: fy },
                    to: golem_events::Point { x: tx, y: ty },
                    result: golem_events::ScrollAttemptResult::Stall { count: stall_count, max: max_stalls },
                    tree_stats: iter_stats,
                });
            }
            continue;
        }

        // Stall limit reached. Reverse direction.
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
                let vis_top = cb.y.max(0);
                let vis_bot = (cb.y + cb.height).min(viewport.height);
                let vis_cx = (cb.x.max(0) + (cb.x + cb.width).min(viewport.width)) / 2;
                container_start = Some(match direction {
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
                });
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
            let vis_top = cb.y.max(0);
            let vis_bot = (cb.y + cb.height).min(viewport.height);
            let vis_cx = (cb.x.max(0) + (cb.x + cb.width).min(viewport.width)) / 2;
            container_start = Some(match direction {
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
            });
        }
    }

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
            visible_bounds: None,
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
        async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<()> {
            self.record_call("launch_app", vec![bundle_id.to_string()]); Ok(())
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
        async fn set_orientation(&self, orientation: &str) -> anyhow::Result<()> {
            self.record_call("set_orientation", vec![orientation.to_string()]); Ok(())
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
            self.record_call("gesture", vec![format!("{} fingers", fingers.len())]); Ok(())
        }
    }

    // ── 1. Element found in initial hierarchy (no scroll needed) ─────

    #[tokio::test]
    async fn element_found_in_initial_hierarchy() {
        let mut root = make_element("View", default_bounds());
        root.children.push(make_element_with_text("Button", "Target", Bounds::new(10, 10, 100, 44)));

        let driver = MockPlatformDriver::new(root);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20, None, None, None)
            .await
            .expect("should find element without scrolling");

        assert_eq!(result.element.text.as_deref(), Some("Target"));
        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
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

        let result = scroll_to_element(&selector, &driver, Direction::Down, 20, None, None, None)
            .await
            .expect("should find element after one scroll");

        assert_eq!(result.element.text.as_deref(), Some("Target"));
        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(scroll_intent(&swipe_calls[0].1), "Down");
    }

    // ── 3. Element not found after max_scrolls → error ──────────────

    #[tokio::test]
    async fn element_not_found_after_max_scrolls() {
        let hierarchies: Vec<Element> = (0..25)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text("Label", &format!("Page {i}"), Bounds::new(0, 0, 200, 40)));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Nonexistent");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 5, None, None, None).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("not found after 5 scroll attempts"),
            "error should mention max scrolls, got: {err_msg}"
        );
    }

    // ── 4. Bounce detection triggers direction reversal ─────────────

    #[tokio::test]
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

        let _ = scroll_to_element(&selector, &driver, Direction::Down, 30, None, None, None).await;

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
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

    #[tokio::test]
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

        let result = scroll_to_element(&selector, &driver, Direction::Down, 30, None, None, None)
            .await
            .expect("should find element after direction reversal");

        assert_eq!(result.element.text.as_deref(), Some("Target"));

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
        let directions: Vec<&str> = swipe_calls.iter().map(|c| scroll_intent(&c.1)).collect();
        assert!(
            directions.contains(&"Up"),
            "should reverse and find target, got: {directions:?}"
        );
    }

    // ── 6. Max scrolls reached returns appropriate error ────────────

    #[tokio::test]
    async fn max_scrolls_reached_returns_error() {
        let hierarchies: Vec<Element> = (0..10)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text("Label", &format!("Screen {i}"), Bounds::new(0, 0, 200, 40)));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Missing");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 3, None, None, None).await;
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
        let root = make_element("View", default_bounds());
        let driver = MockPlatformDriver::new(root);
        let selector = sel_with_text("Anything");

        let result = scroll_to_element(&selector, &driver, Direction::Down, 3, None, None, None).await;
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

        scroll_to_element(&selector, &driver, direction, 20, None, None, None)
            .await
            .expect("should find element");

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
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

    // ── 12. Default max_scrolls behavior ────────────────────────────

    #[tokio::test]
    async fn default_max_scrolls_behavior() {
        assert_eq!(DEFAULT_MAX_SCROLLS, 20);

        let hierarchies: Vec<Element> = (0..25)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text("Label", &format!("Screen {i}"), Bounds::new(0, 0, 200, 40)));
                root
            })
            .collect();

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Nonexistent");

        let result = scroll_to_element(&selector, &driver, Direction::Down, DEFAULT_MAX_SCROLLS, None, None, None).await;
        assert!(result.is_err());

        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), DEFAULT_MAX_SCROLLS as usize);
    }

    // ── 13. Element found on last allowed scroll ────────────────────

    #[tokio::test]
    async fn element_found_on_last_allowed_scroll() {
        let max_scrolls = 3_u32;
        let mut hierarchies: Vec<Element> = (0..3)
            .map(|i| {
                let mut root = make_element("View", default_bounds());
                root.children.push(make_element_with_text("Label", &format!("Page {i}"), Bounds::new(0, 0, 200, 40)));
                root
            })
            .collect();

        let mut target_root = make_element("View", default_bounds());
        target_root.children.push(make_element_with_text("Button", "Target", Bounds::new(10, 10, 100, 44)));
        hierarchies.push(target_root);

        let driver = SequenceMockDriver::new(hierarchies);
        let selector = sel_with_text("Target");

        let result = scroll_to_element(&selector, &driver, Direction::Down, max_scrolls, None, None, None)
            .await
            .expect("should find element on last scroll");

        assert_eq!(result.element.text.as_deref(), Some("Target"));
        let swipe_calls: Vec<_> = driver.get_calls().into_iter().filter(|(m, _)| m == "swipe_coords").collect();
        assert_eq!(swipe_calls.len(), max_scrolls as usize);
    }

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
}
