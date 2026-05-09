use std::time::Instant;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::{AppConfig, Step};

use crate::context::ExecutionContext;
use crate::resolution::wait_for_settle;

/// Resolve the app identifier from a step to a bundle ID.
///
/// The step's `app` field can be either:
/// - A friendly name defined in `[[flow.apps]]` (e.g. `"app"`) — resolved to the bundle ID.
/// - A bundle ID directly (e.g. `"fail.golem.test"`) — used as-is.
///
/// If `apps` is empty or the name doesn't match, the value is treated as a bundle ID.
pub fn resolve_app_bundle<'a>(step: &'a Step, apps: &'a [AppConfig]) -> Result<&'a str> {
    let app_ref = step
        .app
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No app specified for {} action", step.action))?;

    // Try to resolve as a friendly name first.
    if let Some(config) = apps.iter().find(|a| a.name == app_ref) {
        return config.bundle.as_deref()
            .ok_or_else(|| anyhow::anyhow!(
                "app '{}' has no bundle id — add one to [[flow.apps]] or to [[apps]] in golem.toml",
                config.name));
    }

    // Fall back to treating it as a direct bundle ID.
    Ok(app_ref)
}

/// Launch the app with the given bundle_id. Records wall-clock launch timing
/// on the execution context when perf capture is active.
pub(crate) async fn handle_launch(
    step: &Step,
    driver: &dyn PlatformDriver,
    apps: &[AppConfig],
    ctx: &ExecutionContext<'_>,
) -> Result<()> {
    let bundle_id = resolve_app_bundle(step, apps)?;
    // restart = true: stop first (ignore errors if not running), then launch fresh.
    if step.restart == Some(true) {
        let _ = driver.stop_app(bundle_id).await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let start = Instant::now();
    // `launch_app` includes the post-launch settle gate (node-count
    // stability via `await_first_frame`) so the OS-transition pause is
    // already absorbed there. `wait_for_settle` runs after for the
    // additional WebView-enrichment polling it does on top.
    let warning = driver.launch_app(bundle_id).await?;
    let _ = wait_for_settle(driver).await;
    let launch_ms = start.elapsed().as_millis() as u64;
    ctx.substep(golem_events::SubstepEvent::AppLaunch {
        bundle: bundle_id.to_string(),
        duration_ms: launch_ms,
    });
    if let Some(message) = warning {
        ctx.substep(golem_events::SubstepEvent::DriverWarning { message });
    }
    if let Some(collector) = ctx.perf_collector {
        ctx.set_launch_ms(launch_ms);
        collector.set_active(bundle_id);
    }
    Ok(())
}

/// Stop/terminate the app.
pub(crate) async fn handle_stop(
    step: &Step,
    driver: &dyn PlatformDriver,
    apps: &[AppConfig],
    ctx: &ExecutionContext<'_>,
) -> Result<()> {
    let bundle_id = resolve_app_bundle(step, apps)?;
    driver.stop_app(bundle_id).await?;
    ctx.substep(golem_events::SubstepEvent::AppStop {
        bundle: bundle_id.to_string(),
    });
    // Brief pause for the OS to finish terminating the app — no hierarchy
    // fetch needed since the app is gone.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    if let Some(collector) = ctx.perf_collector {
        collector.clear_active(bundle_id);
    }
    Ok(())
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
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use std::path::Path;

    // ── launch action calls driver.launch_app ─────────────────────────

    #[tokio::test]
    async fn launch_action_calls_driver_launch_app() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("launch");
        step.app = Some("com.example.app".to_string());

        handle_launch(&step, &driver, &[], &ctx)
            .await
            .expect("launch should succeed");

        let calls = driver.get_calls();
        let launch_calls: Vec<_> = calls.iter().filter(|c| c.0 == "launch_app").collect();
        assert_eq!(launch_calls.len(), 1);
        assert_eq!(launch_calls[0].1, vec!["com.example.app"]);
    }

    // ── stop action calls driver.stop_app ─────────────────────────────

    // `handle_stop` includes a 2s `sleep` to let the OS finish
    // terminating; under paused-time tokio advances that instantly.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn stop_action_calls_driver_stop_app() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("stop");
        step.app = Some("com.example.app".to_string());

        let ctx = test_ctx(Path::new("."));
        handle_stop(&step, &driver, &[], &ctx)
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

        let ctx = test_ctx(Path::new("."));
        let result = handle_launch(&step, &driver, &[], &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No app specified"),
            "error should mention no app specified, got: {err_msg}"
        );
    }
}
