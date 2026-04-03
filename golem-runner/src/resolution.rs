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
fn build_selector_from_group(g: &golem_parser::SelectorGroup) -> Selector {
    Selector {
        text: g.text.clone(),
        accessibility_id: g.accessibility_id.clone(),
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
        accessibility_id: g.and_then(|g| g.accessibility_id.clone()).or(step.on_accessibility_id.clone()),
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

/// Build a bounds-only fingerprint of the hierarchy for settle detection.
///
/// Ignores text and accessibility_id so that cursor blinks, live counters,
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
pub(crate) async fn wait_for_settle(driver: &dyn PlatformDriver) -> Result<Element> {
    let deadline = Instant::now() + SETTLE_TIMEOUT;

    let root = driver.get_hierarchy().await?;
    let mut prev_fp = bounds_fingerprint(&root);
    let mut prev_root = root;

    loop {
        if Instant::now() >= deadline {
            return Ok(prev_root);
        }

        tokio::time::sleep(SETTLE_INTERVAL).await;

        let root = match driver.get_hierarchy().await {
            Ok(r) => r,
            Err(_) => return Ok(prev_root),
        };
        let fp = bounds_fingerprint(&root);

        if fp == prev_fp {
            return Ok(root);
        }

        prev_fp = fp;
        prev_root = root;
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
) -> Result<(Element, (i32, i32))> {
    let selector = build_selector(step);
    let timeout_ms = step.timeout.unwrap_or(DEFAULT_POLL_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    let auto_scroll = step.auto_scroll == Some(true);

    let (last_root, last_viewport) = loop {
        let root = match driver.get_hierarchy().await {
            Ok(r) => r,
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        let viewport = Viewport::from_root(&root);
        let visible_root = filter_viewport(&root, &viewport);
        let results = find_elements(&visible_root, &selector);

        if !results.is_empty() {
            let first = &results[0];
            return Ok((first.element.clone(), (first.tap_x, first.tap_y)));
        }

        // Element not in viewport — if auto_scroll is set and the element exists
        // off-screen, scroll to it immediately instead of waiting.
        if auto_scroll {
            let full_results = find_elements(&root, &selector);
            if let Some(found) = full_results.first() {
                // Use the element's position to hint direction and distance.
                let elem_y = found.element.bounds.center_y();
                let vp_center = viewport.height / 2;
                let direction = if elem_y > vp_center {
                    golem_driver::Direction::Down
                } else {
                    golem_driver::Direction::Up
                };
                // Distance ratio: how far off-screen (0.0 = near edge, 3.0+ = very far).
                let distance = (elem_y - vp_center).unsigned_abs() as f32
                    / viewport.height as f32;
                let max_scrolls = crate::scroll::DEFAULT_MAX_SCROLLS;
                match crate::scroll::scroll_to_element_with_hint(
                    &selector, driver, direction, max_scrolls, distance,
                ).await {
                    Ok(found) => return Ok((found.element.clone(), (found.tap_x, found.tap_y))),
                    Err(e) => return Err(e),
                }
            }
        }

        if Instant::now() >= deadline {
            break (root, viewport);
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    };

    let elapsed_secs = timeout_ms as f64 / 1000.0;

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
            selector.accessibility_id,
            b.x,
            b.y,
            last_viewport.width,
            last_viewport.height,
        );
    }

    bail!(
        "No element found after {elapsed_secs:.1}s: text={:?}, id={:?}",
        selector.text,
        selector.accessibility_id,
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
    let root = driver.get_hierarchy().await?;
    let results = find_elements(&root, &selector);

    if results.is_empty() {
        bail!(
            "No element found matching selector: text={:?}, id={:?}",
            selector.text,
            selector.accessibility_id,
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
        let root = match driver.get_hierarchy().await {
            Ok(r) => r,
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
                selector.accessibility_id,
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
    use std::collections::HashMap;

    // ── Test helpers ──────────────────────────────────────────────────

    fn make_step(action: &str) -> Step {
        Step {
            action: action.to_string(),
            on_text: None,
            on_accessibility_id: None,
            on_index: None,
            on_enabled: None,
            on_checked: None,
            on_clickable: None,
            on_below: None,
            on_above: None,
            on_right_of: None,
            on_left_of: None,
            on: None,
            input: None,
            if_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            auto_scroll: None,
            params: HashMap::new(),
        }
    }

    fn make_element(element_type: &str, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds,
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

        let (elem, (tap_x, tap_y)) = resolve_element(&step, &driver)
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
        btn.accessibility_id = Some("btn-login".to_string());
        btn.text = Some("Login".to_string());
        root.children.push(btn);

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_accessibility_id = Some("btn-login".to_string());

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find element by id");
        assert_eq!(elem.accessibility_id.as_deref(), Some("btn-login"));
        assert_eq!(elem.text.as_deref(), Some("Login"));
    }

    // ── 3. resolve_element with combined text + accessibility_id ─────

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
        btn.accessibility_id = Some("btn-save".to_string());
        root.children.push(btn);

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Save".to_string());
        step.on_accessibility_id = Some("btn-save".to_string());

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find button with text Save and id btn-save");
        assert_eq!(elem.accessibility_id.as_deref(), Some("btn-save"));
        assert_eq!(elem.text.as_deref(), Some("Save"));
    }

    // ── 4. resolve_element returns error when no elements match ──────

    #[tokio::test]
    async fn resolve_element_no_match_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Nonexistent".to_string());

        let result = resolve_element(&step, &driver).await;
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

        let (elem, (tap_x, tap_y)) = resolve_element(&step, &driver)
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

        let (elem, _coords) = resolve_element(&step, &driver)
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

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find element below Header");
        assert_eq!(elem.text.as_deref(), Some("Below"));
    }

    // ── 8. build_selector maps all step fields correctly ─────────────

    #[test]
    fn build_selector_maps_all_fields() {
        let mut step = make_step("tap");
        step.on_text = Some("Submit".to_string());
        step.on_accessibility_id = Some("btn-1".to_string());
        step.on_index = Some(2);
        step.on_enabled = Some(true);
        step.on_checked = Some(false);
        step.on_clickable = Some(true);
        step.on_below = Some("Header".to_string());
        step.on_above = Some("Footer".to_string());
        step.on_right_of = Some("Sidebar".to_string());
        step.on_left_of = Some("Panel".to_string());

        let sel = build_selector(&step);
        assert_eq!(sel.text.as_deref(), Some("Submit"));
        assert_eq!(sel.accessibility_id.as_deref(), Some("btn-1"));
        assert_eq!(sel.index, Some(2));
        assert_eq!(sel.enabled, Some(true));
        assert_eq!(sel.checked, Some(false));
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
        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find element with glob");
        assert_eq!(elem.text.as_deref(), Some("Item 1"));
    }

    // ── 10. resolve_element with enabled/checked/clickable filters ───

    #[tokio::test]
    async fn resolve_element_with_state_filters() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));

        let mut enabled_checked = make_element_with_text(
            "Checkbox",
            "Option A",
            Bounds::new(0, 0, 100, 30),
        );
        enabled_checked.enabled = true;
        enabled_checked.checked = true;
        enabled_checked.clickable = true;

        let mut enabled_unchecked = make_element_with_text(
            "Checkbox",
            "Option B",
            Bounds::new(0, 40, 100, 30),
        );
        enabled_unchecked.enabled = true;
        enabled_unchecked.checked = false;
        enabled_unchecked.clickable = true;

        let mut disabled_checked = make_element_with_text(
            "Checkbox",
            "Option C",
            Bounds::new(0, 80, 100, 30),
        );
        disabled_checked.enabled = false;
        disabled_checked.checked = true;
        disabled_checked.clickable = false;

        root.children.push(enabled_checked);
        root.children.push(enabled_unchecked);
        root.children.push(disabled_checked);

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.on_text = Some("Option *".to_string());
        step.on_enabled = Some(true);
        step.on_checked = Some(true);
        step.on_clickable = Some(true);

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find enabled, checked, clickable checkbox");
        assert_eq!(elem.text.as_deref(), Some("Option A"));
        assert!(elem.enabled);
        assert!(elem.checked);
        assert!(elem.clickable);
    }
}
