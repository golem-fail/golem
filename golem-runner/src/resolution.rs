use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::selector::{find_elements, AnchorSelector, Selector};
use golem_element::{filter_viewport, Element, Viewport};
use golem_parser::{Anchor, Step};
use tokio::time::Instant;

/// Default timeout for polling the hierarchy when resolving elements (10 seconds).
const DEFAULT_POLL_TIMEOUT_MS: u64 = 10_000;

/// Interval between poll attempts (250ms).
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Maximum time to wait for the UI hierarchy to stabilize (1.5 seconds).
const SETTLE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Interval between settle comparison checks (250ms).
const SETTLE_INTERVAL: Duration = Duration::from_millis(250);

/// Build a `Selector` from the fields of a parsed `Step`.
///
/// Maps each optional selector/filter field on the step to the
/// corresponding field on `Selector`. Fields that are `None` on the
/// step remain `None` on the selector (i.e. not constrained).
/// Build a `Selector` from the step's selector fields.
///
/// Supports three syntaxes:
/// - Flat: `on_text = "Submit"`, `on_below = "Counter"`
/// - Grouped: `on = { text = "Submit", below = "Counter" }`
/// - To alias: `to = { text = "Item 49" }`
///
/// Grouped fields take precedence over flat fields.
/// Convert a parser `Anchor` to a runtime `AnchorSelector`.
fn convert_anchor(anchor: &Anchor) -> AnchorSelector {
    match anchor {
        Anchor::Text(s) => AnchorSelector::Text(s.clone()),
        Anchor::Selector(group) => AnchorSelector::Full(Box::new(build_selector_from_group(group))),
    }
}

/// Build a `Selector` from a `SelectorGroup` (recursive for nested anchors).
pub fn build_selector_from_group(g: &golem_parser::SelectorGroup) -> Selector {
    Selector {
        text: g.text.clone(),
        accessibility_label: g.accessibility_label.clone(),
        index: g.index,
        enabled: g.enabled,
        checked: g.checked,
        clickable: g.clickable,
        below: g.below.as_ref().map(convert_anchor),
        above: g.above.as_ref().map(convert_anchor),
        right_of: g.right_of.as_ref().map(convert_anchor),
        left_of: g.left_of.as_ref().map(convert_anchor),
        traits: g.traits.clone(),
    }
}

/// Build a `Selector` from the step's selector fields.
///
/// Supports flat `on_*`, grouped `on = {}`, `to = {}`, and nested anchors.
/// Grouped fields take precedence over flat fields.
pub fn build_selector(step: &Step) -> Selector {
    let g = step.on.as_ref();
    Selector {
        text: g.and_then(|g| g.text.clone()).or(step.on_text.clone()),
        accessibility_label: g.and_then(|g| g.accessibility_label.clone()).or(step.on_accessibility_label.clone()),
        index: g.and_then(|g| g.index).or(step.on_index),
        enabled: g.and_then(|g| g.enabled).or(step.on_enabled),
        checked: g.and_then(|g| g.checked).or(step.on_checked),
        clickable: g.and_then(|g| g.clickable).or(step.on_clickable),
        below: g.and_then(|g| g.below.as_ref().map(convert_anchor))
            .or(step.on_below.as_ref().map(|s| AnchorSelector::Text(s.clone()))),
        above: g.and_then(|g| g.above.as_ref().map(convert_anchor))
            .or(step.on_above.as_ref().map(|s| AnchorSelector::Text(s.clone()))),
        right_of: g.and_then(|g| g.right_of.as_ref().map(convert_anchor))
            .or(step.on_right_of.as_ref().map(|s| AnchorSelector::Text(s.clone()))),
        left_of: g.and_then(|g| g.left_of.as_ref().map(convert_anchor))
            .or(step.on_left_of.as_ref().map(|s| AnchorSelector::Text(s.clone()))),
        traits: g.map(|g| g.traits.clone()).unwrap_or_default(),
    }
}

/// Build a human-readable label for a selector (for event output).
fn selector_label(sel: &Selector) -> String {
    if let Some(ref t) = sel.text { return t.clone(); }
    if let Some(ref a) = sel.accessibility_label { return a.clone(); }
    "?".to_string()
}

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
    resolve_coord(val, viewport_size, element_pos, element_size, element_origin)
}

fn resolve_coord(
    val: &golem_parser::CoordValue,
    viewport_size: i32,
    element_pos: Option<i32>,   // element center position
    element_size: Option<i32>,  // element width or height
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
fn apply_coord_adjustments(
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
        (Some(b.center_x()), Some(b.center_y()), Some(b.width), Some(b.height), Some(b.x), Some(b.y))
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
fn safe_tap_coords(
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
    let safe_bottom = vis_bottom.min(viewport.y + viewport.height - safe_area_bottom).min(viewport.y + viewport.height - TAP_MARGIN);
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

/// Build a bounds-only fingerprint of the hierarchy for settle detection.
///
/// Ignores text and accessibility_label so that cursor blinks, live counters,
/// and other content changes don't prevent settling. Only structural and
/// spatial changes (animations, scroll momentum, layout shifts) count.
fn bounds_fingerprint(element: &Element) -> String {
    let mut buf = String::with_capacity(256);
    build_bounds_fingerprint(element, &mut buf);
    buf
}

fn build_bounds_fingerprint(element: &Element, buf: &mut String) {
    buf.push_str(&element.element_type);
    let b = &element.bounds;
    buf.push_str(&format!("@{},{},{}x{}", b.x, b.y, b.width, b.height));
    buf.push('[');
    for child in &element.children {
        build_bounds_fingerprint(child, buf);
        buf.push(',');
    }
    buf.push(']');
}

/// Wait for the UI hierarchy to stabilize before acting on it.
///
/// Compares consecutive hierarchy snapshots using a bounds-only fingerprint.
/// Returns the settled hierarchy when two consecutive snapshots match, or
/// the latest snapshot if the settle timeout is exceeded (never fails).
///
/// When the UI is already stable, this completes in a single extra hierarchy
/// fetch (~250ms). During animations it waits up to `SETTLE_TIMEOUT` (1.5s).
/// Maximum time to wait for WebView enrichment after settle (10 seconds).
/// Only applies when the tree contains a web_view with no children,
/// indicating WebKit Inspector hasn't connected yet.
const ENRICHMENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Check if the tree contains a web_view element with no children
/// (unenriched — WebKit Inspector hasn't connected yet).
fn has_empty_webview(element: &Element) -> bool {
    if element.element_type == "web_view" && element.children.is_empty() {
        return true;
    }
    element.children.iter().any(has_empty_webview)
}

pub(crate) async fn wait_for_settle(driver: &dyn PlatformDriver) -> Result<(Element, golem_driver::common::HierarchyMeta, golem_events::TreeStats)> {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    let mut stats = golem_events::TreeStats::default();

    let (root, meta) = driver.get_hierarchy().await?;
    stats.record(meta.node_count);
    crate::record_tree_fetch(meta.node_count);
    let mut prev_fp = bounds_fingerprint(&root);
    let mut prev_root = root;
    let mut prev_meta = meta;

    loop {
        if Instant::now() >= deadline {
            // Tree settled but check for unenriched WebView — keep polling
            // until enrichment arrives or enrichment timeout.
            if has_empty_webview(&prev_root) {
                let enrich_deadline = Instant::now() + ENRICHMENT_TIMEOUT;
                while Instant::now() < enrich_deadline {
                    tokio::time::sleep(SETTLE_INTERVAL).await;
                    let (root, meta) = match driver.get_hierarchy().await {
                        Ok(r) => r,
                        Err(_) => break,
                    };
                    stats.record(meta.node_count);
                    crate::record_tree_fetch(meta.node_count);
                    if !has_empty_webview(&root) {
                        // Enrichment arrived — re-settle with enriched tree
                        prev_root = root;
                        prev_meta = meta;
                        // Quick settle check on enriched tree
                        tokio::time::sleep(SETTLE_INTERVAL).await;
                        if let Ok((r2, m2)) = driver.get_hierarchy().await {
                            stats.record(m2.node_count);
                            crate::record_tree_fetch(m2.node_count);
                            return Ok((r2, m2, stats));
                        }
                        return Ok((prev_root, prev_meta, stats));
                    }
                }
            }
            return Ok((prev_root, prev_meta, stats));
        }

        tokio::time::sleep(SETTLE_INTERVAL).await;

        let (root, meta) = match driver.get_hierarchy().await {
            Ok(r) => r,
            Err(_) => return Ok((prev_root, prev_meta, stats)),
        };
        stats.record(meta.node_count);
        crate::record_tree_fetch(meta.node_count);
        let fp = bounds_fingerprint(&root);

        if fp == prev_fp {
            // Settled — but if web_view is empty, keep polling for enrichment
            if has_empty_webview(&root) {
                prev_root = root;
                prev_meta = meta;
                prev_fp = fp;
                continue; // don't return yet, wait for enrichment
            }
            return Ok((root, meta, stats));
        }

        prev_fp = fp;
        prev_root = root;
        prev_meta = meta;
    }
}

/// Resolve an element from the **viewport-filtered** hierarchy, polling until
/// found or timeout.
///
/// Only elements whose bounds intersect the screen viewport are considered.
/// This matches how a real user interacts — you can only tap what you can see.
///
/// Each poll iteration waits for the UI to settle before checking, preventing
/// ghost taps on animating elements. Polls every 250ms for up to
/// `step.timeout` (default 10s).
///
/// If the element is not found in the viewport but exists in the full tree,
/// the error message includes a hint about its off-screen location.
pub async fn resolve_element(
    step: &Step,
    driver: &dyn PlatformDriver,
    emitter: Option<&golem_events::emitter::DeviceEmitter>,
) -> Result<(Element, (i32, i32))> {
    let selector = build_selector(step);
    let timeout_ms = step.timeout.unwrap_or(DEFAULT_POLL_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    let auto_scroll = step.auto_scroll == Some(true);

    // Handle coordinate-only selector: { x = 150, y = 300 } or { x = "50%", y = "25%" }
    // No element resolution needed — just return the coordinates.
    let has_element_selector = selector.text.is_some()
        || selector.accessibility_label.is_some()
        || selector.below.is_some()
        || selector.above.is_some()
        || selector.right_of.is_some()
        || selector.left_of.is_some();

    if !has_element_selector {
        let group = step.on.as_ref();
        let has_coords = group.is_some_and(|g| g.x.is_some() || g.y.is_some());
        if has_coords {
            let (root, meta) = driver.get_hierarchy().await?;
            crate::record_tree_fetch(meta.node_count);
            let mut vp = Viewport::from_root(&root);
            if meta.keyboard_height > 0 { vp.height -= meta.keyboard_height; }
            let (x, y) = apply_coord_adjustments(step, vp.width / 2, vp.height / 2, &vp, None);
            let dummy = golem_element::Element {
                element_type: "point".to_string(),
                text: None,
                accessibility_label: None,
                placeholder: None,
                enabled: true,
                checked: false,
                clickable: true,
                focused: false,
                bounds: golem_element::Bounds::new(x, y, 1, 1),
                visible_bounds: None,
                children: vec![],
            };
            return Ok((dummy, (x, y)));
        }
    }

    let (last_root, last_viewport) = loop {
        let (root, meta) = match driver.get_hierarchy().await {
            Ok((root, meta)) => { crate::record_tree_fetch(meta.node_count); (root, meta) },
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        // Reduce viewport by keyboard height — elements behind the keyboard
        // are not visible to the user.
        let mut viewport = Viewport::from_root(&root);
        if meta.keyboard_height > 0 {
            viewport.height -= meta.keyboard_height;
        }
        let visible_root = filter_viewport(&root, &viewport);
        let results = find_elements(&visible_root, &selector);

        if !results.is_empty() {
            let first = &results[0];
            let base = safe_tap_coords(first.element.effective_bounds(), &viewport, meta.safe_area_top, meta.safe_area_bottom.max(meta.keyboard_height))
                .unwrap_or((first.tap_x, first.tap_y));
            let coords = apply_coord_adjustments(step, base.0, base.1, &viewport, Some(first.element.effective_bounds()));
            if let Some(e) = emitter {
                let eb = first.element.effective_bounds();
                e.substep(golem_events::SubstepEvent::ElementResolved {
                    selector: selector_label(&selector),
                    bounds: golem_events::Rect { x: eb.x, y: eb.y, width: eb.width, height: eb.height },
                    tap_point: golem_events::Point { x: coords.0, y: coords.1 },
                });
            }
            return Ok((first.element.clone(), coords));
        }

        // Element not in viewport — if auto_scroll is set, scroll to find it.
        if auto_scroll {
            // Resolve `within` container — scroll to it first if off-screen.
            let container_bounds = if let Some(ref within_group) = step.within {
                let within_sel = build_selector_from_group(within_group);
                let in_viewport = find_elements(&visible_root, &within_sel)
                    .first()
                    .map(|r| r.element.bounds);

                let resolve_within = |bounds: Option<golem_element::Bounds>| bounds;

                if let Some(ref b) = in_viewport {
                    // Container visible — nudge to get more of it on screen
                    crate::actions::interaction::nudge_into_view(driver, b, &viewport).await;
                    let (r, m) = driver.get_hierarchy().await?;
                    crate::record_tree_fetch(m.node_count);
                    let mut v = Viewport::from_root(&r);
                    if m.keyboard_height > 0 { v.height -= m.keyboard_height; }
                    let vis = filter_viewport(&r, &v);
                    resolve_within(find_elements(&vis, &within_sel)
                        .first()
                        .map(|r| r.element.bounds)
                        .or(in_viewport))
                } else {
                    // Container not visible — scroll the page to bring
                    // it into view. Timeout + stall govern; no attempt cap.
                    let _ = crate::scroll::scroll_to_element(
                        &within_sel, driver, golem_driver::Direction::Down,
                        None, None, emitter,
                    ).await;
                    let (fresh_root, fresh_meta) = driver.get_hierarchy().await?;
                    crate::record_tree_fetch(fresh_meta.node_count);
                    let mut fresh_vp = Viewport::from_root(&fresh_root);
                    if fresh_meta.keyboard_height > 0 {
                        fresh_vp.height -= fresh_meta.keyboard_height;
                    }
                    let fresh_visible = filter_viewport(&fresh_root, &fresh_vp);
                    let bounds = find_elements(&fresh_visible, &within_sel)
                        .first()
                        .map(|r| r.element.bounds);
                    if let Some(ref b) = bounds {
                        crate::actions::interaction::nudge_into_view(driver, b, &fresh_vp).await;
                        let (r2, m2) = driver.get_hierarchy().await?;
                        crate::record_tree_fetch(m2.node_count);
                        let mut v2 = Viewport::from_root(&r2);
                        if m2.keyboard_height > 0 { v2.height -= m2.keyboard_height; }
                        let vis2 = filter_viewport(&r2, &v2);
                        resolve_within(find_elements(&vis2, &within_sel)
                            .first()
                            .map(|r| r.element.bounds)
                            .or(bounds))
                    } else {
                        resolve_within(bounds)
                    }
                }
            } else {
                None
            };

            // Use position hints from the full tree to determine direction.
            // If the element isn't in the tree at all (e.g. Android WebView
            // accessibility gap), default to scrolling down.
            let full_results = find_elements(&root, &selector);
            let direction = if let Some(found) = full_results.first() {
                let elem_y = found.element.bounds.center_y();
                let ref_center = container_bounds.as_ref()
                    .map(|b| b.center_y())
                    .unwrap_or(viewport.height / 2);
                if elem_y > ref_center {
                    golem_driver::Direction::Down
                } else {
                    golem_driver::Direction::Up
                }
            } else {
                golem_driver::Direction::Down
            };

            match crate::scroll::scroll_to_element(
                &selector, driver, direction,
                step.scroll_timeout, container_bounds, emitter,
            ).await {
                Ok(found) => {
                    let coords = safe_tap_coords(found.element.effective_bounds(), &viewport, meta.safe_area_top, meta.safe_area_bottom.max(meta.keyboard_height))
                        .unwrap_or((found.tap_x, found.tap_y));
                    return Ok((found.element.clone(), coords));
                }
                Err(e) => return Err(e),
            }
        }

        if Instant::now() >= deadline {
            break (root, viewport);
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    };

    let elapsed_secs = timeout_ms as f64 / 1000.0;

    if let Some(e) = emitter {
        e.substep(golem_events::SubstepEvent::ElementNotFound {
            selector: selector_label(&selector),
            timeout_ms,
        });
    }

    // Check full tree for a better error message.
    let full_results = find_elements(&last_root, &selector);
    if !full_results.is_empty() {
        let offscreen = &full_results[0].element;
        let b = &offscreen.bounds;
        bail!(
            "Element not in viewport after {elapsed_secs:.1}s (text={:?}, id={:?}): \
             found off-screen at ({}, {}), viewport {}x{}. \
             Use auto_scroll = true to scroll to off-screen elements.",
            selector.text,
            selector.accessibility_label,
            b.x,
            b.y,
            last_viewport.width,
            last_viewport.height,
        );
    }

    bail!(
        "No element found after {elapsed_secs:.1}s: text={:?}, id={:?}",
        selector.text,
        selector.accessibility_label,
    );
}

/// Resolve an element from the **full** hierarchy (not viewport-filtered).
///
/// Used by actions that need to find off-screen elements, like `scroll`.
pub async fn resolve_element_full_tree(
    step: &Step,
    driver: &dyn PlatformDriver,
) -> Result<(Element, (i32, i32))> {
    let selector = build_selector(step);
    let (root, _meta) = driver.get_hierarchy().await?;
    crate::record_tree_fetch(_meta.node_count);
    let results = find_elements(&root, &selector);

    if results.is_empty() {
        bail!(
            "No element found matching selector: text={:?}, id={:?}",
            selector.text,
            selector.accessibility_label,
        );
    }

    let first = &results[0];
    Ok((first.element.clone(), (first.tap_x, first.tap_y)))
}

/// Poll until NO element matches the step's selectors, or timeout.
///
/// Searches the **full** hierarchy (not viewport-filtered) — an element that
/// exists anywhere in the tree counts as present.
///
/// Returns `Ok(())` as soon as the element disappears. If still present at
/// timeout, returns an error. First check runs immediately — zero overhead
/// when the element is already gone.
pub async fn poll_for_absence(
    step: &Step,
    driver: &dyn PlatformDriver,
) -> Result<()> {
    let selector = build_selector(step);
    let timeout_ms = step.timeout.unwrap_or(DEFAULT_POLL_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let (root, _meta) = match driver.get_hierarchy().await {
            Ok((root, meta)) => { crate::record_tree_fetch(meta.node_count); (root, meta) },
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        let results = find_elements(&root, &selector);

        if results.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let elapsed_secs = timeout_ms as f64 / 1000.0;
            bail!(
                "Expected no element matching selector after {elapsed_secs:.1}s, \
                 but found {}: text={:?}, id={:?}",
                results.len(),
                selector.text,
                selector.accessibility_label,
            );
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── Test helpers ──────────────────────────────────────────────────

    fn make_step(action: &str) -> Step {
        Step {
            action: action.to_string(),
            ..Default::default()
        }
    }

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

    // ── 1. resolve_element finds element by text ─────────────────────

    #[tokio::test]
    async fn resolve_element_finds_by_text() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Button",
            "Submit",
            Bounds::new(100, 200, 100, 44),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Cancel",
            Bounds::new(100, 260, 100, 44),
        ));

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());

        let (elem, (tap_x, tap_y)) = resolve_element(&step, &driver, None)
            .await
            .expect("should find element");
        assert_eq!(elem.text.as_deref(), Some("Submit"));
        assert_eq!(tap_x, 150);
        assert_eq!(tap_y, 222);
    }

    // ── 2. resolve_element finds element by id ───────────────────────

    #[tokio::test]
    async fn resolve_element_finds_by_id() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut btn = make_element("Button", Bounds::new(10, 10, 80, 40));
        btn.accessibility_label = Some("btn-login".to_string());
        btn.text = Some("Login".to_string());
        root.children.push(btn);

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_accessibility_label = Some("btn-login".to_string());

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should find element by id");
        assert_eq!(elem.accessibility_label.as_deref(), Some("btn-login"));
        assert_eq!(elem.text.as_deref(), Some("Login"));
    }

    // ── 3. resolve_element with combined text + accessibility_label ─────

    #[tokio::test]
    async fn resolve_element_combined_text_and_id() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        // A Label with text "Save"
        root.children.push(make_element_with_text(
            "Label",
            "Save",
            Bounds::new(10, 10, 80, 30),
        ));
        // A Button with text "Save" and an id
        let mut btn = make_element_with_text(
            "Button",
            "Save",
            Bounds::new(10, 50, 80, 40),
        );
        btn.accessibility_label = Some("btn-save".to_string());
        root.children.push(btn);

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Save".to_string());
        step.on_accessibility_label = Some("btn-save".to_string());

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should find button with text Save and id btn-save");
        assert_eq!(elem.accessibility_label.as_deref(), Some("btn-save"));
        assert_eq!(elem.text.as_deref(), Some("Save"));
    }

    // ── 4. resolve_element returns error when no elements match ──────

    #[tokio::test]
    async fn resolve_element_no_match_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Nonexistent".to_string());
        // Tight test-only timeout: the resolver polls until the deadline
        // before declaring the element missing; without a cap it would
        // wait the full 10s default and slow this test down for no reason.
        step.timeout = Some(50);

        let result = resolve_element(&step, &driver, None).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No element found"),
            "error message should mention no element found, got: {err_msg}"
        );
    }

    // ── 5. resolve_element returns first match when multiple exist ───

    #[tokio::test]
    async fn resolve_element_returns_first_match() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Button",
            "OK",
            Bounds::new(10, 10, 80, 40),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "OK",
            Bounds::new(10, 60, 80, 40),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "OK",
            Bounds::new(10, 110, 80, 40),
        ));

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("OK".to_string());

        let (elem, (tap_x, tap_y)) = resolve_element(&step, &driver, None)
            .await
            .expect("should find first match");
        assert_eq!(elem.text.as_deref(), Some("OK"));
        // First button: center = (10+80/2, 10+40/2) = (50, 30)
        assert_eq!(tap_x, 50);
        assert_eq!(tap_y, 30);
    }

    // ── 6. resolve_element with index selects correct element ────────

    #[tokio::test]
    async fn resolve_element_with_index() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Button",
            "Item A",
            Bounds::new(0, 0, 100, 40),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Item B",
            Bounds::new(0, 50, 100, 40),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Item C",
            Bounds::new(0, 100, 100, 40),
        ));

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Item *".to_string());
        step.on_index = Some(1);

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should find element at index 1");
        assert_eq!(elem.text.as_deref(), Some("Item B"));
    }

    // ── 7. resolve_element with relational selector (below) ──────────

    #[tokio::test]
    async fn resolve_element_with_below() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 600));
        // Header at top: y=0, height=50 => bottom=50
        root.children.push(make_element_with_text(
            "Label",
            "Header",
            Bounds::new(0, 0, 400, 50),
        ));
        // Button above header area (y=10, not below)
        root.children.push(make_element_with_text(
            "Button",
            "Above",
            Bounds::new(0, 10, 100, 30),
        ));
        // Button below header (y=60 > 50)
        root.children.push(make_element_with_text(
            "Button",
            "Below",
            Bounds::new(0, 60, 100, 40),
        ));

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("*".to_string());
        step.on_below = Some("Header".to_string());

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should find element below Header");
        assert_eq!(elem.text.as_deref(), Some("Below"));
    }

    // ── 8. build_selector maps all step fields correctly ─────────────

    #[test]
    fn build_selector_maps_all_fields() {
        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());
        step.on_accessibility_label = Some("btn-1".to_string());
        step.on_index = Some(2);
        step.on_enabled = Some(true);
        step.on_clickable = Some(true);
        step.on_below = Some("Header".to_string());
        step.on_above = Some("Footer".to_string());
        step.on_right_of = Some("Sidebar".to_string());
        step.on_left_of = Some("Panel".to_string());

        let sel = build_selector(&step);
        assert_eq!(sel.text.as_deref(), Some("Submit"));
        assert_eq!(sel.accessibility_label.as_deref(), Some("btn-1"));
        assert_eq!(sel.index, Some(2));
        assert_eq!(sel.enabled, Some(true));
        assert_eq!(sel.clickable, Some(true));
        assert!(matches!(&sel.below, Some(AnchorSelector::Text(s)) if s == "Header"));
        assert!(matches!(&sel.above, Some(AnchorSelector::Text(s)) if s == "Footer"));
        assert!(matches!(&sel.right_of, Some(AnchorSelector::Text(s)) if s == "Sidebar"));
        assert!(matches!(&sel.left_of, Some(AnchorSelector::Text(s)) if s == "Panel"));
    }

    // ── 9. resolve_element with glob pattern in text ─────────────────

    #[tokio::test]
    async fn resolve_element_with_glob_pattern() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Label",
            "Item 1",
            Bounds::new(0, 0, 100, 30),
        ));
        root.children.push(make_element_with_text(
            "Label",
            "Item 2",
            Bounds::new(0, 40, 100, 30),
        ));
        root.children.push(make_element_with_text(
            "Label",
            "Other",
            Bounds::new(0, 80, 100, 30),
        ));

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Item *".to_string());

        // Should return the first of the two "Item *" matches
        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should find element with glob");
        assert_eq!(elem.text.as_deref(), Some("Item 1"));
    }

    // ── 10. resolve_element with enabled/clickable filters ───

    #[tokio::test]
    async fn resolve_element_with_state_filters() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));

        let mut enabled = make_element_with_text(
            "Button",
            "Option A",
            Bounds::new(0, 0, 100, 30),
        );
        enabled.enabled = true;
        enabled.clickable = true;

        let mut disabled = make_element_with_text(
            "Button",
            "Option B",
            Bounds::new(0, 40, 100, 30),
        );
        disabled.enabled = false;
        disabled.clickable = false;

        root.children.push(enabled);
        root.children.push(disabled);

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Option *".to_string());
        step.on_enabled = Some(true);
        step.on_clickable = Some(true);

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should find enabled, clickable button");
        assert_eq!(elem.text.as_deref(), Some("Option A"));
        assert!(elem.enabled);
        assert!(elem.clickable);
    }
}
