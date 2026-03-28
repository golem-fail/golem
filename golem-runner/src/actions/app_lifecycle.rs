use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::{AppConfig, Step};

/// Resolve the app identifier from a step to a bundle ID.
///
/// The step's `app` field can be either:
/// - A friendly name defined in `[[flow.apps]]` (e.g. `"app"`) — resolved to the bundle ID.
/// - A bundle ID directly (e.g. `"com.golem.test"`) — used as-is.
///
/// If `apps` is empty or the name doesn't match, the value is treated as a bundle ID.
pub fn resolve_app_bundle<'a>(step: &'a Step, apps: &'a [AppConfig]) -> Result<&'a str> {
    let app_ref = step
        .app
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No app specified for {} action", step.action))?;

    // Try to resolve as a friendly name first.
    if let Some(config) = apps.iter().find(|a| a.name == app_ref) {
        return Ok(&config.bundle);
    }

    // Fall back to treating it as a direct bundle ID.
    Ok(app_ref)
}

/// Launch the app with the given bundle_id.
pub(crate) async fn handle_launch(
    step: &Step,
    driver: &dyn PlatformDriver,
    apps: &[AppConfig],
) -> Result<()> {
    let bundle_id = resolve_app_bundle(step, apps)?;
    driver.launch_app(bundle_id).await
}

/// Stop/terminate the app.
pub(crate) async fn handle_stop(
    step: &Step,
    driver: &dyn PlatformDriver,
    apps: &[AppConfig],
) -> Result<()> {
    let bundle_id = resolve_app_bundle(step, apps)?;
    driver.stop_app(bundle_id).await
}

/// Clear app data/cache.
pub(crate) async fn handle_clear_data(
    step: &Step,
    driver: &dyn PlatformDriver,
    apps: &[AppConfig],
) -> Result<()> {
    let bundle_id = resolve_app_bundle(step, apps)?;
    driver.clear_app_data(bundle_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── launch action calls driver.launch_app ─────────────────────────

    #[tokio::test]
    async fn launch_action_calls_driver_launch_app() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("launch");
        step.app = Some("com.example.app".to_string());

        handle_launch(&step, &driver, &[])
            .await
            .expect("launch should succeed");

        let calls = driver.get_calls();
        let launch_calls: Vec<_> = calls.iter().filter(|c| c.0 == "launch_app").collect();
        assert_eq!(launch_calls.len(), 1);
        assert_eq!(launch_calls[0].1, vec!["com.example.app"]);
    }

    // ── stop action calls driver.stop_app ─────────────────────────────

    #[tokio::test]
    async fn stop_action_calls_driver_stop_app() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("stop");
        step.app = Some("com.example.app".to_string());

        handle_stop(&step, &driver, &[])
            .await
            .expect("stop should succeed");

        let calls = driver.get_calls();
        let stop_calls: Vec<_> = calls.iter().filter(|c| c.0 == "stop_app").collect();
        assert_eq!(stop_calls.len(), 1);
        assert_eq!(stop_calls[0].1, vec!["com.example.app"]);
    }

    // ── clear_data action calls driver.clear_app_data ─────────────────

    #[tokio::test]
    async fn clear_data_action_calls_driver_clear_app_data() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("clear_data");
        step.app = Some("com.example.app".to_string());

        handle_clear_data(&step, &driver, &[])
            .await
            .expect("clear_data should succeed");

        let calls = driver.get_calls();
        let clear_calls: Vec<_> = calls.iter().filter(|c| c.0 == "clear_app_data").collect();
        assert_eq!(clear_calls.len(), 1);
        assert_eq!(clear_calls[0].1, vec!["com.example.app"]);
    }

    // ── launch without app param returns error ────────────────────────

    #[tokio::test]
    async fn launch_without_app_param_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("launch");
        // No app set

        let result = handle_launch(&step, &driver, &[]).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No app specified"),
            "error should mention no app specified, got: {err_msg}"
        );
    }
}
