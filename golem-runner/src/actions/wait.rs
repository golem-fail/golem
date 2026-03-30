use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::selector::find_elements;
use golem_parser::Step;
use tokio::time::Instant;

use crate::resolution::{build_selector, resolve_element};

/// Wait for an element to appear, polling the hierarchy until found or timeout.
///
/// Default timeout is 10000ms. Poll interval is 500ms.
pub(crate) async fn handle_wait(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let timeout_ms = step.timeout.unwrap_or(10000);
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        match resolve_element(step, driver).await {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(poll_interval).await;
            }
            Err(e) => return Err(anyhow::anyhow!("Timed out waiting for element: {}", e)),
        }
    }
}

/// Wait for an element to disappear, polling the hierarchy until not found or timeout.
///
/// Default timeout is 10000ms. Poll interval is 500ms.
pub(crate) async fn handle_wait_not(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let timeout_ms = step.timeout.unwrap_or(10000);
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let selector = build_selector(step);

    loop {
        let root = driver.get_hierarchy().await?;
        let results = find_elements(&root, &selector);

        if results.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!(
                "Timed out waiting for element to disappear: text={:?}, id={:?}",
                selector.text,
                selector.accessibility_id,
            );
        }

        tokio::time::sleep(poll_interval).await;
    }
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
