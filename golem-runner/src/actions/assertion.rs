use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::glob::glob_match;
use golem_parser::Step;

use crate::resolution::{poll_for_absence, resolve_element};

/// Assert that an element matching the step's selectors exists in the hierarchy.
pub(crate) async fn handle_assert_visible(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    resolve_element(step, driver).await?;
    Ok(())
}

/// Assert that NO element matches the step's selectors.
///
/// Polls the hierarchy until the element disappears or timeout (default 10s).
/// Passes immediately if the element is already absent.
pub(crate) async fn handle_assert_not_visible(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    poll_for_absence(step, driver).await
}


/// Assert that an alert/dialog is currently displayed.
///
/// If the step has a `text` field, the alert element's text is glob-matched
/// against it. If no `text` is provided, any alert satisfies the assertion.
pub(crate) async fn handle_assert_alert(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (root, _meta) = driver.get_hierarchy().await?;
    let alert = golem_driver::common::find_alert(&root)
        .ok_or_else(|| anyhow::anyhow!("assert_alert failed: no alert is displayed"))?;

    if let Some(ref expected_pattern) = step.on_text {
        let alert_text = alert.text.as_deref().unwrap_or("");
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
        step.on_text = Some("Welcome".to_string());

        handle_assert_visible(&step, &driver)
            .await
            .expect("assert_visible should succeed when element exists");
    }

    // ── assert_visible fails when element not found ─────────────────

    #[tokio::test]
    async fn assert_visible_fails_when_element_not_found() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Nonexistent".to_string());

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
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_not_visible");
        step.on_text = Some("Error*".to_string());

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
        step.on_text = Some("Error*".to_string());

        let result = handle_assert_not_visible(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Expected no element"),
            "error should mention unexpected element, got: {err_msg}"
        );
    }

    // ── assert_visible with text selector matches element text ────────

    #[tokio::test]
    async fn assert_visible_with_text_matches_element() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Label",
            "$42.00",
            Bounds::new(50, 100, 200, 30),
        ));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("$42.00".to_string());

        handle_assert_visible(&step, &driver)
            .await
            .expect("assert_visible SHALL succeed when text matches");
    }

    // ── assert_visible with text selector fails on mismatch ─────────

    #[tokio::test]
    async fn assert_visible_with_text_fails_on_mismatch() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Label",
            "$99.99",
            Bounds::new(50, 100, 200, 30),
        ));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("$42.00".to_string());

        let result = handle_assert_visible(&step, &driver).await;
        assert!(result.is_err(), "assert_visible SHALL fail when text does not match");
    }

    // ── assert_visible with enabled selector ─────────────────────────

    #[tokio::test]
    async fn assert_visible_with_enabled_succeeds() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut btn = make_element_with_text("Button", "Submit", Bounds::new(50, 200, 100, 44));
        btn.enabled = true;
        root.children.push(btn);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Submit".to_string());
        step.on_enabled = Some(true);

        handle_assert_visible(&step, &driver)
            .await
            .expect("assert_visible SHALL succeed when element is enabled");
    }

    // ── assert_visible with enabled selector fails when disabled ─────

    #[tokio::test]
    async fn assert_visible_with_enabled_fails_when_disabled() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut btn = make_element_with_text("Button", "Submit", Bounds::new(50, 200, 100, 44));
        btn.enabled = false;
        root.children.push(btn);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Submit".to_string());
        step.on_enabled = Some(true);

        let result = handle_assert_visible(&step, &driver).await;
        assert!(result.is_err(), "assert_visible SHALL fail when element is disabled");
    }

    // ── assert_visible with checked selector ─────────────────────────

    #[tokio::test]
    async fn assert_visible_with_checked_succeeds() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut cb = make_element_with_text("Checkbox", "Agree", Bounds::new(50, 300, 30, 30));
        cb.checked = true;
        root.children.push(cb);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Agree".to_string());
        step.on_checked = Some(true);

        handle_assert_visible(&step, &driver)
            .await
            .expect("assert_visible SHALL succeed when element is checked");
    }

    // ── assert_visible with checked selector fails when unchecked ────

    #[tokio::test]
    async fn assert_visible_with_checked_fails_when_unchecked() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut cb = make_element_with_text("Checkbox", "Agree", Bounds::new(50, 300, 30, 30));
        cb.checked = false;
        root.children.push(cb);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Agree".to_string());
        step.on_checked = Some(true);

        let result = handle_assert_visible(&step, &driver).await;
        assert!(result.is_err(), "assert_visible SHALL fail when element is unchecked");
    }

    // ── assert_alert tests ────────────────────────────────────────────

    #[tokio::test]
    async fn assert_alert_succeeds_when_alert_present() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let alert = make_element_with_text("Alert", "Delete this item?", Bounds::new(50, 200, 275, 150));
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let step = make_step("assert_alert");

        handle_assert_alert(&step, &driver)
            .await
            .expect("assert_alert SHALL succeed when any alert is displayed");
    }

    #[tokio::test]
    async fn assert_alert_with_matching_text_pattern() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let alert = make_element_with_text("Alert", "Delete this item?", Bounds::new(50, 200, 275, 150));
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_alert");
        step.on_text = Some("Delete*".to_string());

        handle_assert_alert(&step, &driver)
            .await
            .expect("assert_alert SHALL succeed when alert text matches glob pattern");
    }

    #[tokio::test]
    async fn assert_alert_fails_with_mismatched_text_pattern() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let alert = make_element_with_text("Alert", "Delete this item?", Bounds::new(50, 200, 275, 150));
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_alert");
        step.on_text = Some("Save*".to_string());

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
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

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
