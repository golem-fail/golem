use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_element::glob::glob_match;
use golem_parser::Step;

use crate::context::ExecutionContext;
use crate::resolution::{poll_for_absence, resolve_element};

/// Assert that an element matching the step's selectors exists in the hierarchy.
pub(crate) async fn handle_assert_visible(step: &Step, driver: &dyn PlatformDriver, ctx: &ExecutionContext<'_>) -> Result<()> {
    resolve_element(step, driver, ctx.emitter).await?;
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
    // Poll for the alert until the surrounding step timeout fires.
    // A one-shot `get_hierarchy()` raced the dialog open animation
    // on busy emulators — `tap "Show Alert"` returned but the
    // accessibility tree hadn't yet reflected the new alert window,
    // so this assertion failed despite the step having seconds of
    // budget left.
    loop {
        let (root, _meta) = driver.get_hierarchy().await?;
        if let Some(alert) = golem_driver::common::find_alert(&root) {
            if let Some(ref expected_pattern) = step.on_text {
                let alert_text = alert.text.as_deref().unwrap_or("");
                if !glob_match(expected_pattern, alert_text) {
                    crate::fail_code!(
                        golem_events::FailureCode::FlowAssertionMismatch,
                        "assert_alert failed: alert text {:?} does not match pattern {:?}",
                        alert_text,
                        expected_pattern,
                    );
                }
            }
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use std::path::Path;

    // ── assert_visible succeeds when element exists ─────────────────

    #[tokio::test]
    async fn assert_visible_succeeds_when_element_exists() {
        let root = root_with_button("Welcome");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Welcome".to_string());

        let ctx = test_ctx(Path::new("."));
        handle_assert_visible(&step, &driver, &ctx)
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
        // Tight test-only timeout: assert_visible polls until the
        // element appears or the deadline; we want fast failure.
        step.timeout = Some(50);

        let ctx = test_ctx(Path::new("."));
        let result = handle_assert_visible(&step, &driver, &ctx).await;
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
        // Tight test-only timeout: assert_not_visible polls until the
        // element disappears or the deadline; with the element fixed in
        // place we just want fast failure.
        step.timeout = Some(50);

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

        let ctx = test_ctx(Path::new("."));
        handle_assert_visible(&step, &driver, &ctx)
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
        // Tight test-only timeout: assert polls until match or deadline.
        step.timeout = Some(50);

        let ctx = test_ctx(Path::new("."));
        let result = handle_assert_visible(&step, &driver, &ctx).await;
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

        let ctx = test_ctx(Path::new("."));
        handle_assert_visible(&step, &driver, &ctx)
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
        // Tight test-only timeout: assert polls for the enabled state
        // until the deadline; we want fast failure.
        step.timeout = Some(50);

        let ctx = test_ctx(Path::new("."));
        let result = handle_assert_visible(&step, &driver, &ctx).await;
        assert!(result.is_err(), "assert_visible SHALL fail when element is disabled");
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

    // ── assert_alert: textless alert matches wildcard pattern ─────────

    #[tokio::test]
    async fn assert_alert_textless_alert_matches_wildcard() {
        // 1. An alert element with no text of its own and no descendant
        //    text leaves `alert.text == None`; the handler treats that as
        //    the empty string, which a `*` pattern SHALL still match.
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let alert = make_element("Alert", Bounds::new(50, 200, 275, 150));
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_alert");
        step.on_text = Some("*".to_string());

        handle_assert_alert(&step, &driver)
            .await
            .expect("assert_alert SHALL succeed when wildcard matches empty alert text");
    }

    // ── assert_alert: textless alert fails a literal pattern ──────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn assert_alert_textless_alert_fails_literal_pattern() {
        // 2. With the alert text resolving to the empty string, a literal
        //    pattern SHALL NOT match, producing the mismatch failure.
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let alert = make_element("Alert", Bounds::new(50, 200, 275, 150));
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("assert_alert");
        step.on_text = Some("Delete*".to_string());

        let result = handle_assert_alert(&step, &driver).await;
        assert!(result.is_err(), "assert_alert SHALL fail when empty text does not match literal");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("does not match"),
            "error SHALL mention pattern mismatch, got: {err_msg}"
        );
    }

    // ── assert_visible: element appears on a later poll ───────────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn assert_visible_succeeds_when_element_appears_on_later_poll() {
        // 1. The element is absent from the first hierarchy snapshot but
        //    present in the steady fallback. resolve_element polls every
        //    250ms, so the SECOND get_hierarchy() call (after the first
        //    miss) SHALL observe the element and the assertion passes.
        //    With paused time the inter-poll sleep advances instantly.
        let absent = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root_with_button("Welcome"));
        // First poll sees a tree without the button; the queue then
        // empties and subsequent polls fall back to the steady tree.
        driver.push_hierarchy(absent);

        let mut step = make_step("assert_visible");
        step.on_text = Some("Welcome".to_string());

        let ctx = test_ctx(Path::new("."));
        handle_assert_visible(&step, &driver, &ctx)
            .await
            .expect("assert_visible SHALL succeed once the element appears on a later poll");

        // Two get_hierarchy calls: the initial miss plus the resolving poll.
        let polls = driver
            .get_calls()
            .iter()
            .filter(|(method, _)| method == "get_hierarchy")
            .count();
        assert!(
            polls >= 2,
            "resolve_element SHALL re-poll after the first miss, got {polls} get_hierarchy calls",
        );
    }

    // ── assert_not_visible: element disappears on a later poll ────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn assert_not_visible_succeeds_when_element_disappears_on_later_poll() {
        // 2. The element is present in the first hierarchy snapshot but
        //    gone from the steady fallback. poll_for_absence keeps polling
        //    every 250ms, so the SECOND get_hierarchy() call SHALL observe
        //    the element gone and the assertion passes (no timeout error).
        let driver = MockPlatformDriver::new(make_element("View", Bounds::new(0, 0, 375, 812)));
        // First poll still sees the element; the queue then empties and the
        // steady (empty) tree shows it has disappeared.
        driver.push_hierarchy(root_with_button("Loading"));

        let mut step = make_step("assert_not_visible");
        step.on_text = Some("Loading".to_string());

        handle_assert_not_visible(&step, &driver)
            .await
            .expect("assert_not_visible SHALL succeed once the element disappears on a later poll");

        let polls = driver
            .get_calls()
            .iter()
            .filter(|(method, _)| method == "get_hierarchy")
            .count();
        assert!(
            polls >= 2,
            "poll_for_absence SHALL re-poll after the first hit, got {polls} get_hierarchy calls",
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn assert_alert_times_out_when_no_alert_displayed() {
        // The action now polls for an alert until the surrounding
        // step timeout fires (no internal deadline). With no outer
        // timeout in the test, wrap the call in a short one — when
        // it fires we know the action was correctly looping rather
        // than bailing on the first miss.
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let step = make_step("assert_alert");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handle_assert_alert(&step, &driver),
        )
        .await;
        assert!(
            result.is_err(),
            "assert_alert SHALL keep polling until cancelled — got {result:?}",
        );
    }
}
