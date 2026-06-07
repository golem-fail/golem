use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;

/// Take a screenshot, optionally saving to a specific path.
pub(crate) async fn handle_screenshot(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let result = driver.screenshot().await?;

    if let Some(path) = step.params.get("path").and_then(|v| v.as_str()) {
        tokio::fs::write(path, &result.data).await?;
    }

    Ok(())
}

/// Push a media file to the device.
pub(crate) async fn handle_add_media(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let path = step
        .params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("add_media action requires 'path' param")))?;
    driver.add_media(path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── screenshot calls driver.screenshot ─────────────────────────────

    #[tokio::test]
    async fn screenshot_calls_driver_screenshot() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("screenshot");

        handle_screenshot(&step, &driver)
            .await
            .expect("screenshot should succeed");

        let calls = driver.get_calls();
        let sc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(sc_calls.len(), 1);
    }

    // ── add_media calls driver.add_media ──────────────────────────────

    #[tokio::test]
    async fn add_media_calls_driver_add_media() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("add_media");
        step.params.insert(
            "path".to_string(),
            toml::Value::String("test_data/photo.jpg".to_string()),
        );

        handle_add_media(&step, &driver)
            .await
            .expect("add_media should succeed");

        let calls = driver.get_calls();
        let am_calls: Vec<_> = calls.iter().filter(|c| c.0 == "add_media").collect();
        assert_eq!(am_calls.len(), 1);
        assert_eq!(am_calls[0].1, vec!["test_data/photo.jpg"]);
    }

    // ── add_media without path param returns error ────────────────────

    #[tokio::test]
    async fn add_media_without_path_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("add_media");
        // No path param

        let result = handle_add_media(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("path"),
            "error should mention path param, got: {err_msg}"
        );
    }
}
