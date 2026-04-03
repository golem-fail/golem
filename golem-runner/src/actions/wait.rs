use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;

use crate::resolution::{poll_for_absence, resolve_element};

/// Wait for an element to appear, polling the hierarchy until found or timeout.
///
/// Default timeout is 10s. Delegates to `resolve_element` which polls internally.
pub(crate) async fn handle_wait(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    resolve_element(step, driver).await?;
    Ok(())
}

/// Wait for an element to disappear, polling the hierarchy until not found or timeout.
///
/// Default timeout is 10s. Delegates to `poll_for_absence`.
pub(crate) async fn handle_wait_not(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    poll_for_absence(step, driver).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── wait succeeds immediately when element present ──────────────

    #[tokio::test]
    async fn wait_succeeds_immediately_when_element_present() {
        let root = root_with_button("Welcome");
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("wait");
        step.on_text = Some("Welcome".to_string());
        step.timeout = Some(1000);

        handle_wait(&step, &driver)
            .await
            .expect("wait should succeed immediately when element is present");
    }

    // ── wait_not succeeds immediately when element absent ───────────

    #[tokio::test]
    async fn wait_not_succeeds_immediately_when_element_absent() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("wait_not");
        step.on_text = Some("Loading...".to_string());
        step.timeout = Some(1000);

        handle_wait_not(&step, &driver)
            .await
            .expect("wait_not should succeed immediately when element is absent");
    }
}
