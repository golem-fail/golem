use std::time::Duration;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_element::selector::find_elements;
use golem_element::{filter_viewport, Element, Viewport};
use golem_parser::Step;
use tokio::time::Instant;

mod bounded;
mod coord;
mod selector_build;
mod settle;

#[cfg(test)]
use golem_element::selector::{AnchorSelector, Selector};
#[cfg(test)]
use golem_parser::Anchor;
#[cfg(test)]
use settle::{bounds_fingerprint, has_empty_webview};

pub(crate) use bounded::{get_hierarchy_bounded, screenshot_bounded, scroll_swipe_bounded};
pub use coord::resolve_coord_public;
pub(crate) use coord::{apply_coord_adjustments, safe_tap_coords};
pub(crate) use selector_build::selector_label;
pub use selector_build::{build_selector, build_selector_from_group};
pub(crate) use settle::{wait_for_settle, wait_for_settle_extended};

/// Default timeout for polling the hierarchy when resolving elements (10 seconds).
const DEFAULT_POLL_TIMEOUT_MS: u64 = 10_000;

/// Interval between poll attempts (250ms).
const POLL_INTERVAL: Duration = Duration::from_millis(250);

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
    // Auto-recovery: when resolution fails because the soft keyboard is
    // up and has occluded the target field, dismiss the keyboard once
    // and retry. Tracked outside the loop so we only attempt it once
    // per resolve call.
    let mut tried_hide_keyboard = false;

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
            let (root, meta) = get_hierarchy_bounded(driver).await?;
            crate::record_tree_fetch(meta.node_count);
            let mut vp = Viewport::from_root(&root);
            if meta.keyboard_height > 0 {
                vp.height -= meta.keyboard_height;
            }
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
                hit_points: vec![],
                drawing_order: None,
                children: vec![],
            };
            return Ok((dummy, (x, y)));
        }
    }

    let (last_root, last_viewport) = loop {
        let (root, meta) = match get_hierarchy_bounded(driver).await {
            Ok((root, meta)) => {
                crate::record_tree_fetch(meta.node_count);
                (root, meta)
            }
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
            let mut base = safe_tap_coords(
                first.element.effective_bounds(),
                &viewport,
                meta.safe_area_top,
                meta.safe_area_bottom.max(meta.keyboard_height),
            )
            .unwrap_or((first.tap_x, first.tap_y));
            // Occlusion-aware: when the element's centre is covered (e.g. by a
            // sticky header) but a sample point is clear, tap the clear point
            // so we hit the target, not the occluder. Only for plain taps —
            // explicit x/y offsets stay centre-relative (handled below) so they
            // remain predictable.
            let has_offset = step
                .on
                .as_ref()
                .is_some_and(|g| g.x.is_some() || g.y.is_some());
            if !has_offset && first.element.center_hittable() == Some(false) {
                base = (first.tap_x, first.tap_y);
            }
            let coords = apply_coord_adjustments(
                step,
                base.0,
                base.1,
                &viewport,
                Some(first.element.effective_bounds()),
            );
            if let Some(e) = emitter {
                let eb = first.element.effective_bounds();
                e.substep(golem_events::SubstepEvent::ElementResolved {
                    selector: selector_label(&selector),
                    bounds: golem_events::Rect {
                        x: eb.x,
                        y: eb.y,
                        width: eb.width,
                        height: eb.height,
                    },
                    tap_point: golem_events::Point {
                        x: coords.0,
                        y: coords.1,
                    },
                });
            }
            return Ok((first.element.clone(), coords));
        }

        // Auto-recovery: target absent from the keyboard-aware viewport
        // but present in the unfiltered DOM. The soft keyboard has eaten
        // the field — dismiss it and re-poll. One-shot per resolve.
        //
        // Android's IME hide is animated + asynchronous: `hide_keyboard`
        // returns immediately but `keyboard_height` reports non-zero
        // until the panel finishes sliding down. Without waiting for
        // that, the re-poll sees the old layout, taps the target at
        // its pre-hide bounds, and the click hits the still-up
        // keyboard — focus stays on the previous field and the typed
        // text appends there. Block on `keyboard_height = 0` (with a
        // timeout) before continuing.
        if !tried_hide_keyboard && meta.keyboard_height > 0 {
            let unfiltered_count = find_elements(&root, &selector).len();
            if golem_common::is_debug() {
                eprintln!(
                    "  [resolver] kb={} filtered=0 unfiltered={} for {:?}",
                    meta.keyboard_height,
                    unfiltered_count,
                    selector_label(&selector)
                );
            }
            if unfiltered_count > 0 {
                tried_hide_keyboard = true;
                let _ = driver.hide_keyboard().await;
                let kb_deadline = Instant::now() + Duration::from_millis(2000);
                while Instant::now() < kb_deadline {
                    if let Ok((_, m)) = driver.get_hierarchy().await {
                        if m.keyboard_height == 0 {
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                continue;
            }
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
                    if m.keyboard_height > 0 {
                        v.height -= m.keyboard_height;
                    }
                    let vis = filter_viewport(&r, &v);
                    resolve_within(
                        find_elements(&vis, &within_sel)
                            .first()
                            .map(|r| r.element.bounds)
                            .or(in_viewport),
                    )
                } else {
                    // Container not visible — scroll the page to bring
                    // it into view. Timeout + stall govern; no attempt cap.
                    let _ = crate::scroll::scroll_to_element(
                        &within_sel,
                        driver,
                        golem_driver::Direction::Down,
                        None,
                        None,
                        emitter,
                        // Container into view — first sighting suffices.
                        0.0,
                    )
                    .await;
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
                        if m2.keyboard_height > 0 {
                            v2.height -= m2.keyboard_height;
                        }
                        let vis2 = filter_viewport(&r2, &v2);
                        resolve_within(
                            find_elements(&vis2, &within_sel)
                                .first()
                                .map(|r| r.element.bounds)
                                .or(bounds),
                        )
                    } else {
                        resolve_within(bounds)
                    }
                }
            } else {
                None
            };

            // Use position hints from the full tree to determine direction.
            // If the element isn't in the tree at all (e.g. Android WebView
            // accessibility gap), fall back to the selector's positional
            // anchor: when `below`/`above`/etc. is set and the anchor is
            // scrolled off-screen, the visibility guard rightly suppresses
            // it from `find_elements`, so we look it up unguarded here just
            // to pick a sane scroll direction. Without this, every such
            // case defaults to Down and we scroll away from an anchor
            // that's actually above the viewport.
            let full_results = find_elements(&root, &selector);
            let direction = if let Some(found) = full_results.first() {
                let elem_y = found.element.bounds.center_y();
                let ref_center = container_bounds
                    .as_ref()
                    .map(|b| b.center_y())
                    .unwrap_or(viewport.height / 2);
                if elem_y > ref_center {
                    golem_driver::Direction::Down
                } else {
                    golem_driver::Direction::Up
                }
            } else if let Some(anchor) = selector
                .below
                .as_ref()
                .or(selector.above.as_ref())
                .or(selector.right_of.as_ref())
                .or(selector.left_of.as_ref())
            {
                match golem_element::selector::resolve_anchor(&root, anchor) {
                    Some(found) => {
                        let y = found.element.bounds.center_y();
                        if y < 0 {
                            golem_driver::Direction::Up
                        } else {
                            golem_driver::Direction::Down
                        }
                    }
                    None => golem_driver::Direction::Down,
                }
            } else {
                golem_driver::Direction::Down
            };

            match crate::scroll::scroll_to_element(
                &selector,
                driver,
                direction,
                step.scroll_timeout,
                container_bounds,
                emitter,
                crate::scroll::target_fraction_from(
                    step.visibility_percentage,
                    container_bounds.is_some(),
                ),
            )
            .await
            {
                Ok(found) => {
                    let coords = safe_tap_coords(
                        found.element.effective_bounds(),
                        &viewport,
                        meta.safe_area_top,
                        meta.safe_area_bottom.max(meta.keyboard_height),
                    )
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
        crate::fail_code!(
            golem_events::FailureCode::FlowElementOffscreen,
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

    crate::fail_code!(
        golem_events::FailureCode::FlowElementNotFound,
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
    let (root, _meta) = get_hierarchy_bounded(driver).await?;
    crate::record_tree_fetch(_meta.node_count);
    let results = find_elements(&root, &selector);

    if results.is_empty() {
        crate::fail_code!(
            golem_events::FailureCode::FlowElementNotFound,
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
pub async fn poll_for_absence(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let selector = build_selector(step);
    let timeout_ms = step.timeout.unwrap_or(DEFAULT_POLL_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let (root, _meta) = match get_hierarchy_bounded(driver).await {
            Ok((root, meta)) => {
                crate::record_tree_fetch(meta.node_count);
                (root, meta)
            }
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
            crate::fail_code!(
                golem_events::FailureCode::FlowUnexpectedlyPresent,
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
            hit_points: vec![],
            drawing_order: None,
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
        let mut btn = make_element_with_text("Button", "Save", Bounds::new(10, 50, 80, 40));
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

    // ── 4b. resolve_element auto-recovers from keyboard occlusion ────

    #[tokio::test]
    async fn resolve_element_auto_hides_keyboard_when_target_occluded() {
        // Simulate: viewport 375x812, keyboard taking the bottom 350px
        // (height 350). A target field at y=600 falls into the
        // keyboard-occluded zone (812 - 350 = 462 viewport bottom).
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Input",
            "Search",
            Bounds::new(10, 600, 80, 40),
        ));
        let driver = MockPlatformDriver::new(root);
        driver.set_keyboard_height(350);

        let mut step = make_step("type");
        step.on_text = Some("Search".to_string());
        step.timeout = Some(500);

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("should auto-hide keyboard and find Search");
        assert_eq!(elem.text.as_deref(), Some("Search"));

        let calls = driver.get_calls();
        let hide_calls: Vec<_> = calls.iter().filter(|c| c.0 == "hide_keyboard").collect();
        assert_eq!(
            hide_calls.len(),
            1,
            "expected exactly one hide_keyboard recovery call, got {}",
            hide_calls.len()
        );
    }

    #[tokio::test]
    async fn resolve_element_does_not_hide_keyboard_when_target_visible() {
        // Same target but the keyboard is much smaller — target at y=200
        // is inside the (812-100)=712 viewport. No recovery needed.
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Input",
            "Search",
            Bounds::new(10, 200, 80, 40),
        ));
        let driver = MockPlatformDriver::new(root);
        driver.set_keyboard_height(100);

        let mut step = make_step("type");
        step.on_text = Some("Search".to_string());
        step.timeout = Some(500);

        let _ = resolve_element(&step, &driver, None)
            .await
            .expect("should find Search without recovery");
        let calls = driver.get_calls();
        assert!(
            !calls.iter().any(|c| c.0 == "hide_keyboard"),
            "hide_keyboard should NOT be called when target is already visible"
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

        let mut enabled = make_element_with_text("Button", "Option A", Bounds::new(0, 0, 100, 30));
        enabled.enabled = true;
        enabled.clickable = true;

        let mut disabled =
            make_element_with_text("Button", "Option B", Bounds::new(0, 40, 100, 30));
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

    // ── 11. resolve_coord: pixel value with no element is absolute ───

    #[test]
    fn resolve_coord_pixels_absolute_without_element() {
        let v = golem_parser::CoordValue::Pixels(150);
        // No element: pixel SHALL be returned as an absolute screen coordinate.
        assert_eq!(resolve_coord_public(&v, 400, None, None, None), 150);
    }

    // ── 12. resolve_coord: pixel value with element is offset from center ─

    #[test]
    fn resolve_coord_pixels_offset_from_element_center() {
        let v = golem_parser::CoordValue::Pixels(10);
        // With an element center at 200, +10px SHALL offset to 210.
        assert_eq!(
            resolve_coord_public(&v, 400, Some(200), Some(80), Some(160)),
            210
        );
        // A negative pixel offset SHALL subtract from the center.
        let neg = golem_parser::CoordValue::Pixels(-30);
        assert_eq!(
            resolve_coord_public(&neg, 400, Some(200), Some(80), Some(160)),
            170
        );
    }

    // ── 13. resolve_coord: percent of viewport without element ──────

    #[test]
    fn resolve_coord_percent_of_viewport() {
        let v = golem_parser::CoordValue::Percent("50%".to_string());
        // 50% of a 400px viewport SHALL be 200.
        assert_eq!(resolve_coord_public(&v, 400, None, None, None), 200);
        // 25% of 400 SHALL be 100.
        let q = golem_parser::CoordValue::Percent("25%".to_string());
        assert_eq!(resolve_coord_public(&q, 400, None, None, None), 100);
    }

    // ── 14. resolve_coord: percent of element dimensions from center ──

    #[test]
    fn resolve_coord_percent_of_element_dimensions() {
        let v = golem_parser::CoordValue::Percent("50%".to_string());
        // element center 200, size 80: 50% SHALL move +40 to the edge => 240.
        assert_eq!(
            resolve_coord_public(&v, 400, Some(200), Some(80), Some(160)),
            240
        );
        // -50% SHALL move to the opposite edge => 160.
        let neg = golem_parser::CoordValue::Percent("-50%".to_string());
        assert_eq!(
            resolve_coord_public(&neg, 400, Some(200), Some(80), Some(160)),
            160
        );
        // 0% SHALL stay at the center.
        let zero = golem_parser::CoordValue::Percent("0%".to_string());
        assert_eq!(
            resolve_coord_public(&zero, 400, Some(200), Some(80), Some(160)),
            200
        );
    }

    // ── 15. resolve_coord: percent falls back to viewport when only pos known ─

    #[test]
    fn resolve_coord_percent_element_pos_without_size_uses_viewport() {
        let v = golem_parser::CoordValue::Percent("50%".to_string());
        // element_pos set but element_size None => percent-of-element branch
        // requires BOTH; SHALL fall back to percent-of-viewport => 200.
        assert_eq!(resolve_coord_public(&v, 400, Some(200), None, None), 200);
    }

    // ── 16. resolve_coord: malformed percent string parses to 0% ─────

    #[test]
    fn resolve_coord_malformed_percent_defaults_to_zero() {
        let v = golem_parser::CoordValue::Percent("abc%".to_string());
        // Unparseable percent SHALL default to 0.0 => 0 of viewport.
        assert_eq!(resolve_coord_public(&v, 400, None, None, None), 0);
        // And 0% of an element SHALL stay at the center.
        assert_eq!(
            resolve_coord_public(&v, 400, Some(200), Some(80), Some(160)),
            200
        );
    }

    // ── 17. build_selector_from_group maps flat fields and traits ────

    #[test]
    fn build_selector_from_group_maps_fields() {
        let g = golem_parser::SelectorGroup {
            text: Some("Submit".to_string()),
            accessibility_label: Some("btn".to_string()),
            index: Some(3),
            enabled: Some(true),
            checked: Some(false),
            clickable: Some(true),
            below: Some(Anchor::Text("Header".to_string())),
            above: None,
            right_of: None,
            left_of: None,
            contains: None,
            inside: None,
            traits: vec!["button".to_string(), "square".to_string()],
            x: None,
            y: None,
        };
        let sel = build_selector_from_group(&g);
        assert_eq!(sel.text.as_deref(), Some("Submit"));
        assert_eq!(sel.accessibility_label.as_deref(), Some("btn"));
        assert_eq!(sel.index, Some(3));
        assert_eq!(sel.enabled, Some(true));
        assert_eq!(sel.checked, Some(false));
        assert_eq!(sel.clickable, Some(true));
        assert!(matches!(&sel.below, Some(AnchorSelector::Text(s)) if s == "Header"));
        assert_eq!(sel.traits, vec!["button".to_string(), "square".to_string()]);
    }

    // ── 18. build_selector_from_group handles a nested-selector anchor ─

    #[test]
    fn build_selector_from_group_nested_anchor() {
        let inner = golem_parser::SelectorGroup {
            text: Some("Theme:".to_string()),
            enabled: Some(true),
            ..Default::default()
        };
        let g = golem_parser::SelectorGroup {
            text: Some("On".to_string()),
            right_of: Some(Anchor::Selector(Box::new(inner))),
            ..Default::default()
        };
        let sel = build_selector_from_group(&g);
        // A nested-selector anchor SHALL convert to AnchorSelector::Full.
        match &sel.right_of {
            Some(AnchorSelector::Full(boxed)) => {
                assert_eq!(boxed.text.as_deref(), Some("Theme:"));
                assert_eq!(boxed.enabled, Some(true));
            }
            other => panic!("expected Full anchor, got {other:?}"),
        }
    }

    // ── 19. build_selector: grouped `on` fields override flat `on_*` ──

    #[test]
    fn build_selector_grouped_overrides_flat() {
        let mut step = make_step("tap");
        step.on_text = Some("FlatText".to_string());
        step.on_index = Some(1);
        step.on = Some(golem_parser::SelectorGroup {
            text: Some("GroupedText".to_string()),
            index: Some(9),
            ..Default::default()
        });
        let sel = build_selector(&step);
        // Grouped fields SHALL take precedence over flat fields.
        assert_eq!(sel.text.as_deref(), Some("GroupedText"));
        assert_eq!(sel.index, Some(9));
    }

    // ── 20. build_selector: flat fields used when group field is None ─

    #[test]
    fn build_selector_falls_back_to_flat_when_group_field_absent() {
        let mut step = make_step("tap");
        step.on_text = Some("FlatText".to_string());
        step.on_below = Some("FlatAnchor".to_string());
        // Group present but without `text`/`below` — flat SHALL fill in.
        step.on = Some(golem_parser::SelectorGroup {
            accessibility_label: Some("grouped-id".to_string()),
            ..Default::default()
        });
        let sel = build_selector(&step);
        assert_eq!(sel.text.as_deref(), Some("FlatText"));
        assert_eq!(sel.accessibility_label.as_deref(), Some("grouped-id"));
        assert!(matches!(&sel.below, Some(AnchorSelector::Text(s)) if s == "FlatAnchor"));
    }

    // ── 21. build_selector: no group means empty traits ─────────────

    #[test]
    fn build_selector_no_group_has_empty_traits() {
        let mut step = make_step("tap");
        step.on_text = Some("X".to_string());
        let sel = build_selector(&step);
        // Without a group, traits SHALL default to empty.
        assert!(sel.traits.is_empty());
    }

    // ── 22. resolve_element: coordinate-only selector returns a point ─

    #[tokio::test]
    async fn resolve_element_coordinate_only_selector() {
        let root = make_element("View", Bounds::new(0, 0, 400, 800));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        // Pure coordinate selector — no text/id/relational, just x/y.
        step.on = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Pixels(150)),
            y: Some(golem_parser::CoordValue::Pixels(300)),
            ..Default::default()
        });
        let (elem, (x, y)) = resolve_element(&step, &driver, None)
            .await
            .expect("coordinate selector SHALL resolve without an element");
        assert_eq!(elem.element_type, "point");
        // Standalone pixels SHALL be absolute coordinates.
        assert_eq!((x, y), (150, 300));
    }

    // ── 23. resolve_element: percent coordinate-only is viewport-relative ─

    #[tokio::test]
    async fn resolve_element_coordinate_only_percent() {
        let root = make_element("View", Bounds::new(0, 0, 400, 800));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on = Some(golem_parser::SelectorGroup {
            x: Some(golem_parser::CoordValue::Percent("50%".to_string())),
            y: Some(golem_parser::CoordValue::Percent("25%".to_string())),
            ..Default::default()
        });
        let (_elem, (x, y)) = resolve_element(&step, &driver, None)
            .await
            .expect("percent coordinate selector SHALL resolve");
        // 50% of 400 width, 25% of 800 height.
        assert_eq!((x, y), (200, 200));
    }

    // ── 24. resolve_element: x-only adjustment keeps base y on a found element ─

    #[tokio::test]
    async fn resolve_element_x_adjustment_offsets_from_element_center() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        // Button center is (50, 30); width 80.
        root.children.push(make_element_with_text(
            "Button",
            "Tap",
            Bounds::new(10, 10, 80, 40),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        // Group carries both a text selector and an x offset.
        step.on = Some(golem_parser::SelectorGroup {
            text: Some("Tap".to_string()),
            x: Some(golem_parser::CoordValue::Percent("50%".to_string())),
            ..Default::default()
        });
        let (_elem, (x, y)) = resolve_element(&step, &driver, None)
            .await
            .expect("should resolve with x adjustment");
        // x = center 50 + 50% of width 80 (=40) => 90.
        assert_eq!(x, 90);
        // y has no adjustment => safe-tap center y = 30.
        assert_eq!(y, 30);
    }

    // ── 25. poll_for_absence returns Ok immediately when element is gone ─

    #[tokio::test]
    async fn poll_for_absence_ok_when_absent() {
        let root = make_element("View", Bounds::new(0, 0, 400, 800));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("assert_not_visible");
        step.on_text = Some("Missing".to_string());
        step.timeout = Some(50);
        poll_for_absence(&step, &driver)
            .await
            .expect("absent element SHALL resolve to Ok");
    }

    // ── 26. poll_for_absence errors when element stays present ───────

    #[tokio::test]
    async fn poll_for_absence_errors_when_present() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        root.children.push(make_element_with_text(
            "Label",
            "StillHere",
            Bounds::new(0, 0, 100, 30),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("assert_not_visible");
        step.on_text = Some("StillHere".to_string());
        step.timeout = Some(50);
        let result = poll_for_absence(&step, &driver).await;
        let err = result.expect_err("present element SHALL error");
        let msg = format!("{err}");
        assert!(
            msg.contains("Expected no element"),
            "error SHALL mention unexpected presence, got: {msg}"
        );
    }

    // ── 27. poll_for_absence finds element off-screen too (full tree) ─

    #[tokio::test]
    async fn poll_for_absence_searches_full_tree_offscreen() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        // Element far off-screen (y = 5000) — viewport filter would hide it,
        // but poll_for_absence searches the full tree, so it counts present.
        root.children.push(make_element_with_text(
            "Label",
            "OffScreen",
            Bounds::new(0, 5000, 100, 30),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("assert_not_visible");
        step.on_text = Some("OffScreen".to_string());
        step.timeout = Some(50);
        let result = poll_for_absence(&step, &driver).await;
        assert!(
            result.is_err(),
            "off-screen element SHALL count as present in full-tree absence poll"
        );
    }

    // ── 28. resolve_element_full_tree finds an off-screen element ────

    #[tokio::test]
    async fn resolve_element_full_tree_finds_offscreen() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        // Off-screen element that resolve_element (viewport-filtered) would miss.
        root.children.push(make_element_with_text(
            "Button",
            "Hidden",
            Bounds::new(10, 9000, 80, 40),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("scroll");
        step.on_text = Some("Hidden".to_string());
        let (elem, (tap_x, tap_y)) = resolve_element_full_tree(&step, &driver)
            .await
            .expect("full-tree resolve SHALL find off-screen element");
        assert_eq!(elem.text.as_deref(), Some("Hidden"));
        // tap point is the element center: (10+40, 9000+20).
        assert_eq!((tap_x, tap_y), (50, 9020));
    }

    // ── 29. resolve_element_full_tree errors when nothing matches ────

    #[tokio::test]
    async fn resolve_element_full_tree_no_match_errors() {
        let root = make_element("View", Bounds::new(0, 0, 400, 800));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("scroll");
        step.on_text = Some("Nope".to_string());
        let result = resolve_element_full_tree(&step, &driver).await;
        let err = result.expect_err("missing element SHALL error");
        assert!(
            format!("{err}").contains("No element found"),
            "error SHALL mention no element found"
        );
    }

    // ── 30. resolve_element off-screen match yields offscreen hint ───

    #[tokio::test]
    async fn resolve_element_offscreen_error_hints_autoscroll() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        // Element exists only far below the viewport; no auto_scroll set.
        root.children.push(make_element_with_text(
            "Button",
            "FarDown",
            Bounds::new(10, 9000, 80, 40),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("FarDown".to_string());
        step.timeout = Some(50);
        let result = resolve_element(&step, &driver, None).await;
        let err = result.expect_err("off-screen element SHALL error without auto_scroll");
        let msg = format!("{err}");
        assert!(
            msg.contains("auto_scroll"),
            "off-screen error SHALL hint at auto_scroll, got: {msg}"
        );
    }

    // ── 31. selector_label prefers text, then a11y label, then '?' ───

    #[test]
    fn selector_label_priority() {
        let mut sel = Selector::default();
        // Empty selector SHALL render as "?".
        assert_eq!(selector_label(&sel), "?");
        // a11y label only.
        sel.accessibility_label = Some("the-id".to_string());
        assert_eq!(selector_label(&sel), "the-id");
        // text takes precedence over a11y label.
        sel.text = Some("Visible".to_string());
        assert_eq!(selector_label(&sel), "Visible");
    }

    // ── 32. safe_tap_coords taps the safe-zone center when partially safe ─

    #[test]
    fn safe_tap_coords_uses_safe_zone_center() {
        let viewport = Viewport::new(400, 800);
        // Element spans the full height; safe zone trims top 100 / bottom 100.
        let bounds = Bounds::new(100, 0, 200, 800);
        let (x, y) = safe_tap_coords(&bounds, &viewport, 100, 100)
            .expect("element with a safe portion SHALL be tappable");
        // x: visible 100..300 (margin-clamped) center = 200.
        assert_eq!(x, 200);
        // safe_top = max(0,100,5)=100; safe_bottom = min(800, 700, 795)=700 => center 400.
        assert_eq!(y, 400);
    }

    // ── 33. safe_tap_coords returns None when visible portion is tiny ─

    #[test]
    fn safe_tap_coords_none_when_too_small() {
        let viewport = Viewport::new(400, 800);
        // Width 3 < TAP_MARGIN (5) => not tappable.
        let bounds = Bounds::new(10, 10, 3, 40);
        assert!(
            safe_tap_coords(&bounds, &viewport, 0, 0).is_none(),
            "sub-margin element SHALL be untappable"
        );
    }

    // ── 34. safe_tap_coords falls back to visible center in danger zone ─

    #[test]
    fn safe_tap_coords_danger_zone_fallback() {
        let viewport = Viewport::new(400, 800);
        // Element fully inside the bottom safe-area (nav bar): y 760..795.
        let bounds = Bounds::new(100, 760, 200, 35);
        // safe_area_bottom 100 pushes safe_bottom to 700, below the element's
        // top — no safe portion — so it SHALL fall back to the visible center.
        let (x, y) = safe_tap_coords(&bounds, &viewport, 0, 100)
            .expect("danger-zone element SHALL still get a tap point");
        // visible 100..300 center 200; vis_top 760, vis_bottom min(795,800)=795 => center 777.
        assert_eq!(x, 200);
        assert_eq!(y, 777);
    }

    // ── 35. has_empty_webview detects a childless web_view anywhere ──

    #[test]
    fn has_empty_webview_detects_unenriched() {
        // Childless web_view at the root.
        let wv = make_element("web_view", Bounds::new(0, 0, 400, 800));
        assert!(has_empty_webview(&wv), "childless web_view SHALL be empty");

        // web_view nested under a container.
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        root.children
            .push(make_element("web_view", Bounds::new(0, 0, 400, 800)));
        assert!(
            has_empty_webview(&root),
            "nested empty web_view SHALL be detected"
        );

        // Enriched web_view (has children) SHALL NOT be flagged.
        let mut enriched = make_element("web_view", Bounds::new(0, 0, 400, 800));
        enriched
            .children
            .push(make_element("div", Bounds::new(0, 0, 100, 50)));
        assert!(
            !has_empty_webview(&enriched),
            "enriched web_view SHALL NOT be empty"
        );

        // Tree with no web_view at all.
        let plain = make_element("View", Bounds::new(0, 0, 400, 800));
        assert!(
            !has_empty_webview(&plain),
            "no web_view SHALL NOT be flagged"
        );
    }

    // ── 36. bounds_fingerprint ignores text but reflects bounds ──────

    #[test]
    fn bounds_fingerprint_ignores_text_tracks_bounds() {
        let a = make_element_with_text("Button", "Live 1", Bounds::new(0, 0, 100, 40));
        let mut b = make_element("Button", Bounds::new(0, 0, 100, 40));
        b.text = Some("Live 2".to_string());
        // Same bounds + type, differing text SHALL fingerprint identically.
        assert_eq!(
            bounds_fingerprint(&a),
            bounds_fingerprint(&b),
            "text changes SHALL NOT affect the bounds fingerprint"
        );

        // A bounds change SHALL change the fingerprint.
        let moved = make_element("Button", Bounds::new(0, 10, 100, 40));
        assert_ne!(
            bounds_fingerprint(&a),
            bounds_fingerprint(&moved),
            "a bounds shift SHALL change the fingerprint"
        );
    }

    // ── 37. bounds_fingerprint reflects child structure ──────────────

    #[test]
    fn bounds_fingerprint_reflects_children() {
        let leaf = make_element("View", Bounds::new(0, 0, 100, 40));
        let mut parent = make_element("View", Bounds::new(0, 0, 100, 40));
        parent
            .children
            .push(make_element("Child", Bounds::new(5, 5, 10, 10)));
        // Adding a child SHALL change the fingerprint.
        assert_ne!(
            bounds_fingerprint(&leaf),
            bounds_fingerprint(&parent),
            "structural changes SHALL change the fingerprint"
        );
    }

    // ── 38. convert_anchor (via group) maps Text and Selector variants ─

    #[test]
    fn build_selector_from_group_text_anchor_variants() {
        // All four relational positions accept a plain text anchor.
        let g = golem_parser::SelectorGroup {
            below: Some(Anchor::Text("B".to_string())),
            above: Some(Anchor::Text("A".to_string())),
            right_of: Some(Anchor::Text("R".to_string())),
            left_of: Some(Anchor::Text("L".to_string())),
            ..Default::default()
        };
        let sel = build_selector_from_group(&g);
        assert!(matches!(&sel.below, Some(AnchorSelector::Text(s)) if s == "B"));
        assert!(matches!(&sel.above, Some(AnchorSelector::Text(s)) if s == "A"));
        assert!(matches!(&sel.right_of, Some(AnchorSelector::Text(s)) if s == "R"));
        assert!(matches!(&sel.left_of, Some(AnchorSelector::Text(s)) if s == "L"));
    }

    // ── 39. resolve_element polls until a later tree contains the target ─

    #[tokio::test]
    async fn resolve_element_polls_until_target_appears() {
        // First two get_hierarchy snapshots lack the target (loading state);
        // the third carries it. The resolver SHALL keep polling across the
        // queue rather than failing on the first empty snapshot.
        let empty = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut populated = make_element("View", Bounds::new(0, 0, 375, 812));
        populated.children.push(make_element_with_text(
            "Button",
            "Submit",
            Bounds::new(100, 200, 100, 44),
        ));

        // Steady fallback also carries the target so the test never hangs to
        // the deadline if the queue drains, but the queue front must be hit.
        let driver = MockPlatformDriver::new(populated.clone());
        driver.push_hierarchy(empty.clone());
        driver.push_hierarchy(empty);
        driver.push_hierarchy(populated);

        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());
        step.timeout = Some(2000);

        let (elem, _coords) = resolve_element(&step, &driver, None)
            .await
            .expect("resolver SHALL find target once a later poll surfaces it");
        assert_eq!(elem.text.as_deref(), Some("Submit"));
        // At least three get_hierarchy calls (two empty + the populated one).
        let fetches = driver
            .get_calls()
            .into_iter()
            .filter(|c| c.0 == "get_hierarchy")
            .count();
        assert!(
            fetches >= 3,
            "resolver SHALL poll the hierarchy repeatedly, got {fetches} fetches"
        );
    }

    // ── 40. resolve_element retries past a transient get_hierarchy error ─

    #[tokio::test]
    async fn resolve_element_retries_after_transient_fetch_error() {
        // The first get_hierarchy errors (companion blip); after the error is
        // cleared a subsequent poll succeeds and finds the target. The
        // resolver's `Err(_) if before deadline` arm SHALL swallow the
        // transient error and retry.
        let mut populated = make_element("View", Bounds::new(0, 0, 375, 812));
        populated.children.push(make_element_with_text(
            "Button",
            "Submit",
            Bounds::new(100, 200, 100, 44),
        ));
        let driver = MockPlatformDriver::new(populated);
        driver.set_error("get_hierarchy", "transient companion blip");

        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());
        step.timeout = Some(2000);

        // Clear the error after a short delay so an in-deadline retry succeeds.
        let driver_ref = &driver;
        let (res, ()) = tokio::join!(resolve_element(&step, driver_ref, None), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            driver_ref.clear_error("get_hierarchy");
        });
        let (elem, _coords) = res.expect("resolver SHALL recover after the transient error clears");
        assert_eq!(elem.text.as_deref(), Some("Submit"));
    }

    // ── 41. resolve_element surfaces a fetch error that never clears ─────

    #[tokio::test]
    async fn resolve_element_propagates_persistent_fetch_error() {
        // get_hierarchy errors for the whole (tight) timeout window. Once the
        // deadline passes the resolver's terminal `Err(e)` arm SHALL return
        // the underlying error rather than a not-found message.
        let driver = MockPlatformDriver::new(make_element("View", Bounds::new(0, 0, 375, 812)));
        driver.set_error("get_hierarchy", "companion wedged hard");

        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());
        step.timeout = Some(50);

        let err = resolve_element(&step, &driver, None)
            .await
            .expect_err("a persistent fetch error SHALL propagate");
        let msg = format!("{err}");
        assert!(
            msg.contains("companion wedged hard"),
            "the underlying fetch error SHALL surface, got: {msg}"
        );
    }

    // ── 42. poll_for_absence waits for the target to disappear ──────────

    #[tokio::test]
    async fn poll_for_absence_waits_for_target_to_disappear() {
        // First snapshot still has the element; a later queued snapshot drops
        // it. poll_for_absence SHALL keep polling and resolve to Ok once gone.
        let mut present = make_element("View", Bounds::new(0, 0, 400, 800));
        present.children.push(make_element_with_text(
            "Label",
            "Spinner",
            Bounds::new(0, 0, 100, 30),
        ));
        let absent = make_element("View", Bounds::new(0, 0, 400, 800));

        // Steady fallback is the absent tree so the queue front (present)
        // is what forces the extra poll.
        let driver = MockPlatformDriver::new(absent.clone());
        driver.push_hierarchy(present.clone());
        driver.push_hierarchy(present);
        driver.push_hierarchy(absent);

        let mut step = make_step("assert_not_visible");
        step.on_text = Some("Spinner".to_string());
        step.timeout = Some(2000);

        poll_for_absence(&step, &driver)
            .await
            .expect("absence poll SHALL succeed once the element disappears");
        let fetches = driver
            .get_calls()
            .into_iter()
            .filter(|c| c.0 == "get_hierarchy")
            .count();
        assert!(
            fetches >= 3,
            "absence poll SHALL re-fetch while the element lingers, got {fetches}"
        );
    }

    // ── 43. poll_for_absence retries past a transient fetch error ───────

    #[tokio::test]
    async fn poll_for_absence_retries_after_transient_fetch_error() {
        // get_hierarchy errors first, then clears. The element is absent in
        // the steady tree, so once a poll succeeds poll_for_absence returns Ok.
        let driver = MockPlatformDriver::new(make_element("View", Bounds::new(0, 0, 400, 800)));
        driver.set_error("get_hierarchy", "transient blip");

        let mut step = make_step("assert_not_visible");
        step.on_text = Some("Spinner".to_string());
        step.timeout = Some(2000);

        let driver_ref = &driver;
        let (res, ()) = tokio::join!(poll_for_absence(&step, driver_ref), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            driver_ref.clear_error("get_hierarchy");
        });
        res.expect("absence poll SHALL recover after the transient fetch error clears");
    }
}
