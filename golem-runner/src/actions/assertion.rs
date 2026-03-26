use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::glob::glob_match;
use golem_element::selector::find_elements;
use golem_element::Element;
use golem_parser::Step;

use crate::resolution::{build_selector, resolve_element};

use super::resolve_element_ignore_text;

/// Collect text from an element's immediate children (typically StaticText nodes).
/// Used for web-based UIs where container elements don't carry text directly.
fn collect_child_text(elem: &Element) -> String {
    let mut parts = Vec::new();
    for child in &elem.children {
        if let Some(text) = child.text.as_deref() {
            if !text.is_empty() {
                parts.push(text);
            }
        }
    }
    parts.join("")
}

/// Assert that an element matching the step's selectors exists in the hierarchy.
pub(crate) async fn handle_assert_visible(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    resolve_element(step, driver).await?;
    Ok(())
}

/// Assert that NO element matches the step's selectors.
pub(crate) async fn handle_assert_not_visible(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let selector = build_selector(step);
    let root = driver.get_hierarchy().await?;
    let results = find_elements(&root, &selector);

    if results.is_empty() {
        Ok(())
    } else {
        bail!(
            "Expected no element matching selector but found {}: text={:?}, id={:?}, type={:?}",
            results.len(),
            selector.text,
            selector.id,
            selector.element_type,
        )
    }
}

/// Assert that an element's text exactly matches the expected value.
///
/// The element is located by `id` (or other non-text selectors).
/// The step's `text` field is used as the expected text to compare against.
pub(crate) async fn handle_assert_text(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let expected = step
        .text
        .as_deref()
        .unwrap_or("");

    // Find element using selectors other than text
    let (elem, _coords) = resolve_element_ignore_text(step, driver).await?;
    // Use the element's own text, or fall back to concatenated child text
    // (needed for web-based UIs where container divs hold text in child nodes).
    let own_text = elem.text.as_deref().unwrap_or("");
    let actual = if own_text.is_empty() {
        collect_child_text(&elem)
    } else {
        own_text.to_string()
    };

    if actual.as_str() == expected {
        Ok(())
    } else {
        bail!(
            "assert_text failed: expected {:?}, got {:?}",
            expected,
            actual,
        )
    }
}

/// Assert that the matched element is enabled.
pub(crate) async fn handle_assert_enabled(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver).await?;
    if elem.enabled {
        Ok(())
    } else {
        bail!(
            "assert_enabled failed: element is disabled (id={:?}, text={:?})",
            elem.id,
            elem.text,
        )
    }
}

/// Assert that the matched element is checked.
pub(crate) async fn handle_assert_checked(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver).await?;
    if elem.checked {
        Ok(())
    } else {
        bail!(
            "assert_checked failed: element is not checked (id={:?}, text={:?})",
            elem.id,
            elem.text,
        )
    }
}

/// Assert that an alert/dialog is currently displayed.
///
/// If the step has a `text` field, the alert element's text is glob-matched
/// against it. If no `text` is provided, any alert satisfies the assertion.
pub(crate) async fn handle_assert_alert(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let alert = driver.get_alert().await?;
    let alert_elem = alert.ok_or_else(|| anyhow::anyhow!("assert_alert failed: no alert is displayed"))?;

    if let Some(ref expected_pattern) = step.text {
        let alert_text = alert_elem.text.as_deref().unwrap_or("");
        if !glob_match(expected_pattern, alert_text) {
            bail!(
                "assert_alert failed: alert text {:?} does not match pattern {:?}",
                alert_text,
                expected_pattern,
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── assert_visible succeeds when element exists ─────────────────

    #[tokio::test]
    async fn assert_visible_succeeds_when_element_exists() {
        let root = root_with_button("Welcome");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.text = Some("Welcome".to_string());

        handle_assert_visible(&step, &driver)
            .await
            .expect("assert_visible should succeed when element exists");
    }

    // ── assert_visible fails when element not found ─────────────────

    #[tokio::test]
    async fn assert_visible_fails_when_element_not_found() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.text = Some("Nonexistent".to_string());

        let result = handle_assert_visible(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No element found"),
            "error should mention no element found, got: {err_msg}"
        );
    }

    // ── assert_not_visible succeeds when element not found ──────────

    #[tokio::test]
    async fn assert_not_visible_succeeds_when_element_not_found() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_not_visible");
        step.text = Some("Error*".to_string());

        handle_assert_not_visible(&step, &driver)
            .await
            .expect("assert_not_visible should succeed when element absent");
    }

    // ── assert_not_visible fails when element exists ────────────────

    #[tokio::test]
    async fn assert_not_visible_fails_when_element_exists() {
        let root = root_with_button("Error occurred");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_not_visible");
        step.text = Some("Error*".to_string());

        let result = handle_assert_not_visible(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Expected no element"),
            "error should mention unexpected element, got: {err_msg}"
        );
    }

    // ── assert_text succeeds when text matches ──────────────────────

    #[tokio::test]
    async fn assert_text_succeeds_when_text_matches() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "total",
            "$42.00",
            Bounds::new(50.0, 100.0, 200.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_text");
        step.id = Some("total".to_string());
        step.text = Some("$42.00".to_string());

        handle_assert_text(&step, &driver)
            .await
            .expect("assert_text should succeed when text matches");
    }

    // ── assert_text fails when text doesn't match ───────────────────

    #[tokio::test]
    async fn assert_text_fails_when_text_does_not_match() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "total",
            "$99.99",
            Bounds::new(50.0, 100.0, 200.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_text");
        step.id = Some("total".to_string());
        step.text = Some("$42.00".to_string());

        let result = handle_assert_text(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("assert_text failed"),
            "error should mention assert_text failed, got: {err_msg}"
        );
        assert!(
            err_msg.contains("$42.00"),
            "error should mention expected value, got: {err_msg}"
        );
        assert!(
            err_msg.contains("$99.99"),
            "error should mention actual value, got: {err_msg}"
        );
    }

    // ── assert_enabled succeeds when element is enabled ─────────────

    #[tokio::test]
    async fn assert_enabled_succeeds_when_element_is_enabled() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut btn = make_element_with_id("Button", "submit-button", Bounds::new(50.0, 200.0, 100.0, 44.0));
        btn.enabled = true;
        root.children.push(btn);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_enabled");
        step.id = Some("submit-button".to_string());

        handle_assert_enabled(&step, &driver)
            .await
            .expect("assert_enabled should succeed when element is enabled");
    }

    // ── assert_enabled fails when element is disabled ───────────────

    #[tokio::test]
    async fn assert_enabled_fails_when_element_is_disabled() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut btn = make_element_with_id("Button", "submit-button", Bounds::new(50.0, 200.0, 100.0, 44.0));
        btn.enabled = false;
        root.children.push(btn);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_enabled");
        step.id = Some("submit-button".to_string());

        let result = handle_assert_enabled(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("assert_enabled failed"),
            "error should mention assert_enabled failed, got: {err_msg}"
        );
    }

    // ── assert_checked succeeds when element is checked ─────────────

    #[tokio::test]
    async fn assert_checked_succeeds_when_element_is_checked() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut cb = make_element_with_id("Checkbox", "agree-checkbox", Bounds::new(50.0, 300.0, 30.0, 30.0));
        cb.checked = true;
        root.children.push(cb);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_checked");
        step.id = Some("agree-checkbox".to_string());

        handle_assert_checked(&step, &driver)
            .await
            .expect("assert_checked should succeed when element is checked");
    }

    // ── assert_checked fails when element is unchecked ──────────────

    #[tokio::test]
    async fn assert_checked_fails_when_element_is_unchecked() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut cb = make_element_with_id("Checkbox", "agree-checkbox", Bounds::new(50.0, 300.0, 30.0, 30.0));
        cb.checked = false;
        root.children.push(cb);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_checked");
        step.id = Some("agree-checkbox".to_string());

        let result = handle_assert_checked(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("assert_checked failed"),
            "error should mention assert_checked failed, got: {err_msg}"
        );
    }

    // ── assert_alert tests ────────────────────────────────────────────

    #[tokio::test]
    async fn assert_alert_succeeds_when_alert_present() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);

        // Set up an alert element
        let alert = make_element_with_text("Alert", "Delete this item?", Bounds::new(50.0, 200.0, 275.0, 150.0));
        driver.set_alert(Some(alert));

        let step = make_step("assert_alert");

        handle_assert_alert(&step, &driver)
            .await
            .expect("assert_alert SHALL succeed when any alert is displayed");
    }

    #[tokio::test]
    async fn assert_alert_with_matching_text_pattern() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);

        let alert = make_element_with_text("Alert", "Delete this item?", Bounds::new(50.0, 200.0, 275.0, 150.0));
        driver.set_alert(Some(alert));

        let mut step = make_step("assert_alert");
        step.text = Some("Delete*".to_string());

        handle_assert_alert(&step, &driver)
            .await
            .expect("assert_alert SHALL succeed when alert text matches glob pattern");
    }

    #[tokio::test]
    async fn assert_alert_fails_with_mismatched_text_pattern() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);

        let alert = make_element_with_text("Alert", "Delete this item?", Bounds::new(50.0, 200.0, 275.0, 150.0));
        driver.set_alert(Some(alert));

        let mut step = make_step("assert_alert");
        step.text = Some("Save*".to_string());

        let result = handle_assert_alert(&step, &driver).await;
        assert!(result.is_err(), "assert_alert SHALL fail when text does not match");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("does not match"),
            "error SHALL mention pattern mismatch, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn assert_alert_fails_when_no_alert_displayed() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        // No alert set -- default is None

        let step = make_step("assert_alert");

        let result = handle_assert_alert(&step, &driver).await;
        assert!(result.is_err(), "assert_alert SHALL fail when no alert is displayed");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("no alert"),
            "error SHALL mention no alert, got: {err_msg}"
        );
    }
}
