use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::selector::{find_elements, Selector};
use golem_element::{filter_viewport, Element, Viewport};
use golem_parser::Step;

/// Build a `Selector` from the fields of a parsed `Step`.
///
/// Maps each optional selector/filter field on the step to the
/// corresponding field on `Selector`. Fields that are `None` on the
/// step remain `None` on the selector (i.e. not constrained).
pub fn build_selector(step: &Step) -> Selector {
    Selector {
        text: step.text.clone(),
        accessibility_id: step.accessibility_id.clone(),
        element_type: step.element_type.clone(),
        index: step.index,
        enabled: step.enabled,
        checked: step.checked,
        clickable: step.clickable,
        placeholder: step.placeholder.clone(),
        below: step.below.clone(),
        above: step.above.clone(),
        right_of: step.right_of.clone(),
        left_of: step.left_of.clone(),
        child_of: step.child_of.clone(),
    }
}

/// Resolve an element from the **viewport-filtered** hierarchy.
///
/// Only elements whose bounds intersect the screen viewport are considered.
/// This matches how a real user interacts — you can only tap what you can see.
///
/// If the element is not found in the viewport but exists in the full tree,
/// the error message includes a hint about its off-screen location.
pub async fn resolve_element(
    step: &Step,
    driver: &dyn PlatformDriver,
) -> Result<(Element, (i32, i32))> {
    let selector = build_selector(step);
    let root = driver.get_hierarchy().await?;
    let viewport = Viewport::from_root(&root);
    let visible_root = filter_viewport(&root, &viewport);
    let results = find_elements(&visible_root, &selector);

    if !results.is_empty() {
        let first = &results[0];
        return Ok((first.element.clone(), (first.tap_x, first.tap_y)));
    }

    // Not found in viewport — check full tree for a better error message.
    let full_results = find_elements(&root, &selector);
    if !full_results.is_empty() {
        let offscreen = &full_results[0].element;
        let b = &offscreen.bounds;
        bail!(
            "Element not in viewport (text={:?}, id={:?}): found off-screen at ({}, {}), viewport {}x{}",
            selector.text,
            selector.accessibility_id,
            b.x,
            b.y,
            viewport.width,
            viewport.height,
        );
    }

    bail!(
        "No element found matching selector: text={:?}, id={:?}, type={:?}",
        selector.text,
        selector.accessibility_id,
        selector.element_type,
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
            "No element found matching selector: text={:?}, id={:?}, type={:?}",
            selector.text,
            selector.accessibility_id,
            selector.element_type,
        );
    }

    let first = &results[0];
    Ok((first.element.clone(), (first.tap_x, first.tap_y)))
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
            text: None,
            accessibility_id: None,
            element_type: None,
            index: None,
            enabled: None,
            checked: None,
            clickable: None,
            below: None,
            above: None,
            right_of: None,
            left_of: None,
            child_of: None,
            placeholder: None,
            on_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
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
        step.text = Some("Submit".to_string());

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
        step.accessibility_id = Some("btn-login".to_string());

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find element by id");
        assert_eq!(elem.accessibility_id.as_deref(), Some("btn-login"));
        assert_eq!(elem.text.as_deref(), Some("Login"));
    }

    // ── 3. resolve_element with combined text + element_type ─────────

    #[tokio::test]
    async fn resolve_element_combined_text_and_type() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        // A Label with text "Save"
        root.children.push(make_element_with_text(
            "Label",
            "Save",
            Bounds::new(10, 10, 80, 30),
        ));
        // A Button with text "Save"
        root.children.push(make_element_with_text(
            "Button",
            "Save",
            Bounds::new(10, 50, 80, 40),
        ));

        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.text = Some("Save".to_string());
        step.element_type = Some("Button".to_string());

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find button with text Save");
        assert_eq!(elem.element_type, "Button");
        assert_eq!(elem.text.as_deref(), Some("Save"));
    }

    // ── 4. resolve_element returns error when no elements match ──────

    #[tokio::test]
    async fn resolve_element_no_match_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut step = make_step("tap");
        step.text = Some("Nonexistent".to_string());

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
        step.text = Some("OK".to_string());

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
        step.element_type = Some("Button".to_string());
        step.index = Some(1);

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
        step.element_type = Some("Button".to_string());
        step.below = Some("Header".to_string());

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find element below Header");
        assert_eq!(elem.text.as_deref(), Some("Below"));
    }

    // ── 8. build_selector maps all step fields correctly ─────────────

    #[test]
    fn build_selector_maps_all_fields() {
        let mut step = make_step("tap");
        step.text = Some("Submit".to_string());
        step.accessibility_id = Some("btn-1".to_string());
        step.element_type = Some("Button".to_string());
        step.index = Some(2);
        step.enabled = Some(true);
        step.checked = Some(false);
        step.clickable = Some(true);
        step.placeholder = Some("Enter text".to_string());
        step.below = Some("Header".to_string());
        step.above = Some("Footer".to_string());
        step.right_of = Some("Sidebar".to_string());
        step.left_of = Some("Panel".to_string());
        step.child_of = Some("Container".to_string());

        let sel = build_selector(&step);
        assert_eq!(sel.text.as_deref(), Some("Submit"));
        assert_eq!(sel.accessibility_id.as_deref(), Some("btn-1"));
        assert_eq!(sel.element_type.as_deref(), Some("Button"));
        assert_eq!(sel.index, Some(2));
        assert_eq!(sel.enabled, Some(true));
        assert_eq!(sel.checked, Some(false));
        assert_eq!(sel.clickable, Some(true));
        assert_eq!(sel.placeholder.as_deref(), Some("Enter text"));
        assert_eq!(sel.below.as_deref(), Some("Header"));
        assert_eq!(sel.above.as_deref(), Some("Footer"));
        assert_eq!(sel.right_of.as_deref(), Some("Sidebar"));
        assert_eq!(sel.left_of.as_deref(), Some("Panel"));
        assert_eq!(sel.child_of.as_deref(), Some("Container"));
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
        step.text = Some("Item *".to_string());

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
        step.element_type = Some("Checkbox".to_string());
        step.enabled = Some(true);
        step.checked = Some(true);
        step.clickable = Some(true);

        let (elem, _coords) = resolve_element(&step, &driver)
            .await
            .expect("should find enabled, checked, clickable checkbox");
        assert_eq!(elem.text.as_deref(), Some("Option A"));
        assert!(elem.enabled);
        assert!(elem.checked);
        assert!(elem.clickable);
    }
}
