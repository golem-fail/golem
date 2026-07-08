use std::time::Instant;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_events::CodeExt;
use golem_parser::{AppConfig, Step};

use crate::context::ExecutionContext;

/// Resolve the app identifier from a step to a bundle ID.
///
/// The step's `app` field can be either:
/// - A friendly name defined in `[[flow.apps]]` (e.g. `"app"`) — resolved to the bundle ID.
/// - A bundle ID directly (e.g. `"fail.golem.test"`) — used as-is.
///
/// If `apps` is empty or the name doesn't match, the value is treated as a bundle ID.
pub(super) fn resolve_app_bundle<'a>(step: &'a Step, apps: &'a [AppConfig]) -> Result<&'a str> {
    let app_ref = step.app.as_deref().ok_or_else(|| {
        golem_events::coded(
            golem_events::FailureCode::ParseMissingParam,
            anyhow::anyhow!("No app specified for {} action", step.action),
        )
    })?;

    // Try to resolve as a friendly name first.
    if let Some(config) = apps.iter().find(|a| a.name == app_ref) {
        return config.bundle.as_deref().ok_or_else(|| {
            golem_events::coded(
                golem_events::FailureCode::ParseMissingParam,
                anyhow::anyhow!(
                "app '{}' has no bundle id — add one to [[flow.apps]] or to [[apps]] in golem.toml",
                config.name),
            )
        });
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
    // already absorbed there. Additional WebView-enrichment settle
    // runs out-of-band after this step returns (see policy.rs's
    // `needs_post_settle`), not inline — otherwise the launch step's
    // own timeout would absorb the wait.
    let warning = driver
        .launch_app(bundle_id)
        .await
        .code(golem_events::FailureCode::AppLifecycleFailed)?;
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
    driver
        .stop_app(bundle_id)
        .await
        .code(golem_events::FailureCode::AppLifecycleFailed)?;
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
    use crate::context::{test_ctx, TestHarness};
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use golem_events::{extract_code, EventKind, FailureCode, SubstepEvent};
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

    // ── resolve_app_bundle ────────────────────────────────────────────

    fn app_config(name: &str, bundle: Option<&str>) -> AppConfig {
        AppConfig {
            name: name.to_string(),
            bundle: bundle.map(str::to_string),
            devices: Vec::new(),
            install_script: None,
            install_timeout_ms: None,
            install_env: None,
            profile: None,
        }
    }

    // 1. A friendly name matching an apps entry SHALL resolve to that
    //    entry's bundle id, not the friendly name.
    #[test]
    fn resolve_app_bundle_resolves_friendly_name_to_bundle() {
        let mut step = make_step("launch");
        step.app = Some("app".to_string());
        let apps = vec![app_config("app", Some("com.example.real"))];

        let resolved = resolve_app_bundle(&step, &apps).expect("friendly name SHALL resolve");
        assert_eq!(
            resolved, "com.example.real",
            "friendly name SHALL resolve to the configured bundle id"
        );
    }

    // 2. A matching friendly name whose entry has no bundle id SHALL be
    //    an error naming the offending app.
    #[test]
    fn resolve_app_bundle_errors_when_matched_app_has_no_bundle() {
        let mut step = make_step("launch");
        step.app = Some("myFriendlyApp".to_string());
        let apps = vec![app_config("myFriendlyApp", None)];

        let err = resolve_app_bundle(&step, &apps).expect_err("missing bundle SHALL error");
        let msg = format!("{err}");
        // A distinctive name proves the offending app id is interpolated,
        // not merely that the boilerplate "app '...'" text is present.
        assert!(
            msg.contains("has no bundle id") && msg.contains("'myFriendlyApp'"),
            "error SHALL name the offending app missing a bundle id, got: {msg}"
        );
    }

    // 3. An app value that matches no apps entry SHALL fall back to being
    //    treated as a direct bundle id (even when apps is non-empty).
    #[test]
    fn resolve_app_bundle_falls_back_to_direct_bundle_id() {
        let mut step = make_step("launch");
        step.app = Some("direct.bundle.id".to_string());
        let apps = vec![app_config("other", Some("com.example.other"))];

        let resolved =
            resolve_app_bundle(&step, &apps).expect("unmatched value SHALL pass through");
        assert_eq!(
            resolved, "direct.bundle.id",
            "an unmatched app value SHALL be used verbatim as the bundle id"
        );
    }

    // 4. The first matching apps entry SHALL win when names collide.
    #[test]
    fn resolve_app_bundle_uses_first_matching_entry() {
        let mut step = make_step("launch");
        step.app = Some("app".to_string());
        let apps = vec![
            app_config("app", Some("com.first")),
            app_config("app", Some("com.second")),
        ];

        let resolved = resolve_app_bundle(&step, &apps).expect("duplicate names SHALL resolve");
        assert_eq!(
            resolved, "com.first",
            "the first matching apps entry SHALL win"
        );
    }

    // 5. A missing app value SHALL error and the message SHALL name the
    //    offending action.
    #[test]
    fn resolve_app_bundle_error_names_action() {
        let step = make_step("clear_data");
        let err = resolve_app_bundle(&step, &[]).expect_err("missing app SHALL error");
        let msg = format!("{err}");
        assert!(
            msg.contains("No app specified") && msg.contains("clear_data"),
            "error SHALL name the action with no app, got: {msg}"
        );
    }

    // ── launch with restart stops first, then launches ────────────────

    // `handle_launch` with restart=true issues a stop_app before launch;
    // it also sleeps 1s, so run under paused time to keep this fast.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn launch_with_restart_stops_then_launches() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("launch");
        step.app = Some("com.example.app".to_string());
        step.restart = Some(true);

        handle_launch(&step, &driver, &[], &ctx)
            .await
            .expect("restart launch SHALL succeed");

        let calls = driver.get_calls();
        let names: Vec<&str> = calls.iter().map(|c| c.0.as_str()).collect();
        let stop_pos = names
            .iter()
            .position(|&n| n == "stop_app")
            .expect("restart SHALL issue a stop_app");
        let launch_pos = names
            .iter()
            .position(|&n| n == "launch_app")
            .expect("restart SHALL still launch_app");
        assert!(
            stop_pos < launch_pos,
            "stop_app SHALL precede launch_app on restart"
        );
    }

    // ── launch resolves friendly name before launching ────────────────

    // 7. handle_launch SHALL launch the resolved bundle id, not the
    //    friendly name, when an apps entry matches.
    #[tokio::test]
    async fn launch_resolves_friendly_name_before_launching() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("launch");
        step.app = Some("app".to_string());
        let apps = vec![app_config("app", Some("com.example.resolved"))];

        handle_launch(&step, &driver, &apps, &ctx)
            .await
            .expect("launch SHALL succeed");

        let calls = driver.get_calls();
        let launch_calls: Vec<_> = calls.iter().filter(|c| c.0 == "launch_app").collect();
        assert_eq!(launch_calls.len(), 1, "exactly one launch_app SHALL fire");
        assert_eq!(
            launch_calls[0].1,
            vec!["com.example.resolved"],
            "launch SHALL use the resolved bundle id"
        );
    }

    // ── launch without perf collector does not record launch_ms ───────

    // 8. With no perf collector active, handle_launch SHALL NOT store a
    //    launch duration on the context.
    #[tokio::test]
    async fn launch_without_perf_collector_does_not_store_launch_ms() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("launch");
        step.app = Some("com.example.app".to_string());

        handle_launch(&step, &driver, &[], &ctx)
            .await
            .expect("launch SHALL succeed");

        assert_eq!(
            ctx.take_launch_ms(),
            None,
            "no perf collector SHALL mean no recorded launch_ms"
        );
    }

    // ── failure propagation (#35) ─────────────────────────────────────

    // 9. A failing driver.launch_app SHALL propagate as an error tagged
    //    AppLifecycleFailed, with the driver message preserved.
    #[tokio::test]
    async fn launch_failure_propagates_with_app_lifecycle_code() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        driver.set_error("launch_app", "device offline");
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("launch");
        step.app = Some("com.example.app".to_string());

        let err = handle_launch(&step, &driver, &[], &ctx)
            .await
            .expect_err("launch_app failure SHALL propagate as an error");
        assert_eq!(
            extract_code(&err),
            Some(FailureCode::AppLifecycleFailed),
            "launch failure SHALL be tagged AppLifecycleFailed"
        );
        let msg = format!("{err:#}");
        assert!(
            msg.contains("device offline"),
            "launch failure SHALL preserve the driver message, got: {msg}"
        );
    }

    // 10. A failing driver.stop_app SHALL propagate as an error tagged
    //     AppLifecycleFailed, with the driver message preserved.
    //
    // `handle_stop` only sleeps 2s *after* a successful stop, so the
    // failing path returns immediately — no paused-time flavor needed.
    #[tokio::test]
    async fn stop_failure_propagates_with_app_lifecycle_code() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        driver.set_error("stop_app", "terminate refused");
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("stop");
        step.app = Some("com.example.app".to_string());

        let err = handle_stop(&step, &driver, &[], &ctx)
            .await
            .expect_err("stop_app failure SHALL propagate as an error");
        assert_eq!(
            extract_code(&err),
            Some(FailureCode::AppLifecycleFailed),
            "stop failure SHALL be tagged AppLifecycleFailed"
        );
        let msg = format!("{err:#}");
        assert!(
            msg.contains("terminate refused"),
            "stop failure SHALL preserve the driver message, got: {msg}"
        );
    }

    // 11. A failing driver.clear_app_data SHALL propagate the raw driver
    //     error verbatim (handle_clear_data does not re-tag it).
    #[tokio::test]
    async fn clear_data_failure_propagates_driver_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        // "clear_data" is the documented shorthand for clear_app_data.
        driver.set_error("clear_data", "wipe denied");

        let mut step = make_step("clear_data");
        step.app = Some("com.example.app".to_string());

        let err = handle_clear_data(&step, &driver, &[])
            .await
            .expect_err("clear_app_data failure SHALL propagate as an error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("wipe denied"),
            "clear_data failure SHALL surface the driver message, got: {msg}"
        );
    }

    // 12. When launch fails, the success-path substeps SHALL NOT fire and
    //     the perf collector SHALL NOT be marked active for the bundle.
    #[tokio::test]
    async fn launch_failure_does_not_emit_substeps_or_activate_perf() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        driver.set_error("launch_app", "boom");

        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = crate::perf::RawPerfData::default();
        let mut harness = TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        {
            let ctx = harness.ctx();
            let mut step = make_step("launch");
            step.app = Some("com.example.app".to_string());

            handle_launch(&step, &driver, &[], &ctx)
                .await
                .expect_err("launch_app failure SHALL propagate");

            // launch_ms is only stored on the perf-success path.
            assert_eq!(
                ctx.take_launch_ms(),
                None,
                "a failed launch SHALL NOT record launch_ms"
            );
        }
        // No AppLaunch / DriverWarning substep SHALL have been emitted.
        while let Some(event) = harness.try_recv() {
            assert!(
                !matches!(
                    event.kind,
                    EventKind::Substep(SubstepEvent::AppLaunch { .. })
                        | EventKind::Substep(SubstepEvent::DriverWarning { .. })
                ),
                "a failed launch SHALL NOT emit AppLaunch or DriverWarning substeps"
            );
        }
    }

    // ── launch-warning surfacing (#36) ────────────────────────────────

    // 13. A non-fatal launch warning (Ok(Some(_))) SHALL surface as a
    //     DriverWarning substep while the launch step still succeeds.
    #[tokio::test]
    async fn launch_warning_surfaces_as_driver_warning_substep() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        driver.set_launch_warning("settle-probe timed out");

        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = crate::perf::RawPerfData::default();
        let mut harness = TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        {
            let ctx = harness.ctx();
            let mut step = make_step("launch");
            step.app = Some("com.example.app".to_string());

            handle_launch(&step, &driver, &[], &ctx)
                .await
                .expect("a launch warning SHALL NOT fail the step");
        }

        let mut saw_warning = false;
        while let Some(event) = harness.try_recv() {
            if let EventKind::Substep(SubstepEvent::DriverWarning { message }) = event.kind {
                assert_eq!(
                    message, "settle-probe timed out",
                    "the surfaced warning SHALL carry the driver's message"
                );
                saw_warning = true;
            }
        }
        assert!(
            saw_warning,
            "a launch warning SHALL surface as a DriverWarning substep"
        );
    }

    // 14. With no launch warning set, launch SHALL NOT emit any
    //     DriverWarning substep.
    #[tokio::test]
    async fn launch_without_warning_emits_no_driver_warning() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = crate::perf::RawPerfData::default();
        let mut harness = TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        {
            let ctx = harness.ctx();
            let mut step = make_step("launch");
            step.app = Some("com.example.app".to_string());

            handle_launch(&step, &driver, &[], &ctx)
                .await
                .expect("launch SHALL succeed");
        }

        while let Some(event) = harness.try_recv() {
            assert!(
                !matches!(
                    event.kind,
                    EventKind::Substep(SubstepEvent::DriverWarning { .. })
                ),
                "no warning SHALL mean no DriverWarning substep"
            );
        }
    }
}
