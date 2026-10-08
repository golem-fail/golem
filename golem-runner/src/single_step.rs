//! Run one step outside a flow, for interactive sessions.

use std::time::Instant;

use golem_driver::PlatformDriver;
use golem_parser::{AppConfig, Step};
use golem_report::{StepOutcome as ReportOutcome, StepReport, SubstepDetail};
use golem_vars::VariableStore;

use crate::context::ExecutionContext;
use crate::executor::{interpolate_for_run, step_target};
use crate::policy::{execute_step_with_policy, StepOutcome};

/// Run `step` against `driver` through the same interpolation, timeout,
/// retry, settle and element resolution as a step inside a flow, and
/// report it.
///
/// Each call advances `ctx.global_step_index`, so consecutive calls on one
/// context number their steps in order. `${var}` references resolve from
/// `vars`, and a step that saves a value (`read`, `save_to`) writes it
/// there for the next call. `apps` is the registry `launch`/`stop`/`app=`
/// resolve against; its first entry is the app `${_app}` names.
///
/// A step that cannot be interpolated is reported as failed, without
/// running. With `ctx.recovery`, a companion that dies during the step is
/// restarted, and the step is retried unless its mutation already landed,
/// as in a flow.
pub async fn execute_single_step(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    ctx: &mut ExecutionContext<'_>,
    apps: &[AppConfig],
    timeout_ms: u64,
) -> StepReport {
    ctx.step_index = usize::try_from(ctx.global_step_index).unwrap_or(usize::MAX);
    ctx.global_step_index += 1;
    let mut report = StepReport {
        global_step_index: ctx.global_step_index,
        block_name: ctx.block_name.unwrap_or_default().to_string(),
        block_iteration: ctx.block_iteration,
        step_index_in_block: ctx.step_index,
        action: step.action.clone(),
        target: step_target(step),
        outcome: ReportOutcome::Success,
        skip_reason: None,
        duration_ms: 0,
        retry_count: 0,
        screenshot_path: None,
        substeps: Vec::new(),
        tree_stats: golem_events::TreeStats::default(),
        started_at: None,
        finished_at: None,
    };

    let primary_app = apps.first().map(|a| a.name.as_str());
    let step = match interpolate_for_run(step, vars, ctx, primary_app, None) {
        Ok(step) => step,
        Err(e) => {
            report.outcome = failed(&e);
            return report;
        }
    };
    report.target = step_target(&step);

    ctx.emit(golem_events::EventKind::StepStarted {
        global_step_index: ctx.global_step_index,
        block_name: report.block_name.clone(),
        step_index_in_block: ctx.step_index,
        action: step.action.clone(),
        selector_label: report.target.clone(),
    });
    if let Ok(mut log) = ctx.substep_log.lock() {
        *log = Some(Vec::new());
    }
    crate::reset_step_tree_stats();
    let started = Instant::now();

    let witness = ctx
        .recovery
        .map(|_| crate::recovery::WitnessDriver::new(driver));
    let step_driver: &dyn PlatformDriver = match witness.as_ref() {
        Some(w) => {
            w.set_terminal(crate::recovery::terminal_mutation(&step.action));
            w
        }
        None => driver,
    };
    let result = execute_step_with_policy(&step, step_driver, vars, timeout_ms, ctx, apps).await;
    let result = match (ctx.recovery, witness.as_ref()) {
        (Some(recovery), Some(w)) => {
            crate::executor::run_step_recovery(
                result,
                recovery,
                w,
                step_driver,
                &step,
                vars,
                timeout_ms,
                ctx,
                apps,
                &mut 0,
            )
            .await
        }
        _ => result,
    };
    let result = crate::policy::apply_if_fail_for_death(result, &step);

    report.duration_ms = started.elapsed().as_millis() as u64;
    report.tree_stats = crate::take_step_tree_stats();
    report.substeps = ctx
        .substep_log
        .lock()
        .ok()
        .and_then(|mut log| log.take())
        .unwrap_or_default()
        .iter()
        .map(SubstepDetail::from)
        .collect();

    let event_outcome = match result {
        Ok(StepOutcome::Success) => golem_events::StepOutcome::Success,
        Ok(StepOutcome::Warning { message, code }) => {
            report.outcome = ReportOutcome::Warning {
                message: message.clone(),
                code,
            };
            golem_events::StepOutcome::Warning { message, code }
        }
        Ok(StepOutcome::Ignored { message, code }) => {
            report.outcome = ReportOutcome::Skipped;
            report.skip_reason = Some(format!("{}: {message}", code.fragment()));
            golem_events::StepOutcome::Ignored { message, code }
        }
        Err(e) => {
            report.outcome = failed(&e);
            let code = golem_events::extract_code(&e).unwrap_or(golem_events::FailureCode::Uncoded);
            golem_events::StepOutcome::Failed {
                message: golem_events::clean_msg(&e),
                code,
            }
        }
    };
    ctx.emit(golem_events::EventKind::StepFinished {
        global_step_index: ctx.global_step_index,
        outcome: event_outcome,
        duration_ms: report.duration_ms,
        retry_count: 0,
        screenshot_path: None,
        tree_stats: report.tree_stats,
    });
    report
}

fn failed(e: &anyhow::Error) -> ReportOutcome {
    ReportOutcome::Failed {
        message: golem_events::clean_msg(e),
        code: golem_events::extract_code(e).unwrap_or(golem_events::FailureCode::Uncoded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use golem_report::toon::format_step_toon;
    use golem_vars::VarValue;

    fn screen() -> MockPlatformDriver {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_text(
            "Button",
            "Sign in",
            Bounds::new(40, 400, 200, 50),
        ));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "otp-code",
            "123456",
            Bounds::new(40, 500, 200, 30),
        ));
        MockPlatformDriver::new(root)
    }

    fn tap(text: &str) -> Step {
        Step {
            on_text: Some(text.into()),
            ..make_step("tap")
        }
    }

    #[tokio::test]
    async fn a_passing_step_reports_success_with_where_it_tapped() {
        let driver = screen();
        let mut vars = make_vars();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());

        let report =
            execute_single_step(&tap("Sign in"), &driver, &mut vars, &mut ctx, &[], 2_000).await;

        assert!(matches!(report.outcome, ReportOutcome::Success));
        assert_eq!(report.global_step_index, 1);
        assert!(
            report
                .substeps
                .iter()
                .any(|s| matches!(s, SubstepDetail::Tap { .. })),
            "the tap SHALL be in the substeps: {:?}",
            report.substeps.len()
        );
        let line = format_step_toon(&report);
        assert!(line.starts_with(r#" +tap:on_text="Sign in" d:"#), "{line}");
        assert!(line.contains("@140,425"), "{line}");
    }

    #[tokio::test]
    async fn a_failing_step_reports_the_failure_and_its_code() {
        let driver = screen();
        let mut vars = make_vars();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());
        let mut fail = make_step("fail");
        fail.params.insert(
            "message".into(),
            toml::Value::String("cart is empty".into()),
        );

        let report = execute_single_step(&fail, &driver, &mut vars, &mut ctx, &[], 2_000).await;

        match &report.outcome {
            ReportOutcome::Failed { code, message } => {
                assert_eq!(*code, golem_events::FailureCode::FlowExplicitFail);
                assert_eq!(message, "cart is empty");
            }
            _ => panic!("SHALL fail: {}", format_step_toon(&report)),
        }
        let line = format_step_toon(&report);
        assert!(line.starts_with(" !fail d:"), "{line}");
        assert!(line.ends_with("cart is empty"), "{line}");
    }

    #[tokio::test]
    async fn variables_interpolate_and_carry_across_calls() {
        let driver = screen();
        let mut vars = make_vars();
        vars.set_in_scope(
            golem_vars::ScopeLevel::Flow,
            "button",
            VarValue::string("Sign in"),
        );
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());

        let report =
            execute_single_step(&tap("${button}"), &driver, &mut vars, &mut ctx, &[], 2_000).await;
        assert!(matches!(report.outcome, ReportOutcome::Success));
        assert_eq!(report.target, r#"on_text="Sign in""#);

        let read = Step {
            on_accessibility_label: Some("otp-code".into()),
            save_to: Some("otp".into()),
            ..make_step("read")
        };
        let report = execute_single_step(&read, &driver, &mut vars, &mut ctx, &[], 2_000).await;
        assert!(matches!(report.outcome, ReportOutcome::Success));
        assert_eq!(report.global_step_index, 2);
        assert_eq!(vars.get("otp"), Some(&VarValue::string("123456")));

        // The value `read` saved resolves in the next call.
        let assert = Step {
            on_text: Some("${otp}".into()),
            ..make_step("assert_visible")
        };
        let report = execute_single_step(&assert, &driver, &mut vars, &mut ctx, &[], 2_000).await;
        assert!(matches!(report.outcome, ReportOutcome::Success));
        assert_eq!(report.target, r#"on_text="123456""#);
    }

    #[tokio::test]
    async fn an_undefined_variable_fails_without_running() {
        let driver = screen();
        let mut vars = make_vars();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());

        let report =
            execute_single_step(&tap("${nope}"), &driver, &mut vars, &mut ctx, &[], 2_000).await;

        match &report.outcome {
            ReportOutcome::Failed { code, .. } => {
                assert_eq!(*code, golem_events::FailureCode::ParseVariable);
            }
            _ => panic!("SHALL fail: {}", format_step_toon(&report)),
        }
        assert!(
            !driver.get_calls().iter().any(|(m, _)| m == "tap"),
            "nothing SHALL reach the device"
        );
    }

    /// Counts restarts; the mock driver's next call after a scripted death
    /// succeeds, so a restart needs to change nothing.
    struct CountingRecovery(std::sync::atomic::AtomicU32);

    #[async_trait::async_trait]
    impl crate::recovery::CompanionRecovery for CountingRecovery {
        async fn restart_and_reconnect(&self) -> anyhow::Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn taps(driver: &MockPlatformDriver) -> usize {
        driver.get_calls().iter().filter(|c| c.0 == "tap").count()
    }

    #[tokio::test]
    async fn a_companion_death_before_the_tap_restarts_and_retries_the_step() {
        let driver = screen();
        driver.set_error_on_calls(
            "tap",
            golem_events::FailureCode::DeviceCompanionUnreachable,
            "connection refused",
            &[1],
        );
        let recovery = CountingRecovery(Default::default());
        let mut vars = make_vars();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());
        ctx.recovery = Some(&recovery);

        let report =
            execute_single_step(&tap("Sign in"), &driver, &mut vars, &mut ctx, &[], 2_000).await;

        assert!(
            matches!(report.outcome, ReportOutcome::Success),
            "{}",
            format_step_toon(&report)
        );
        assert_eq!(recovery.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            taps(&driver),
            2,
            "the step SHALL run again after the restart"
        );
        assert!(
            report
                .substeps
                .iter()
                .any(|s| matches!(s, SubstepDetail::CompanionRestarted { .. })),
            "the report SHALL show the restart"
        );
    }

    #[tokio::test]
    async fn a_companion_death_after_the_tap_landed_does_not_tap_again() {
        let driver = screen();
        // get_hierarchy #1 checks the keyboard, #2 resolves, then the tap
        // lands, and #3 (the post-settle read) dies.
        driver.set_error_on_calls(
            "get_hierarchy",
            golem_events::FailureCode::DeviceCompanionUnreachable,
            "connection refused",
            &[3],
        );
        let recovery = CountingRecovery(Default::default());
        let mut vars = make_vars();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());
        ctx.recovery = Some(&recovery);

        let report =
            execute_single_step(&tap("Sign in"), &driver, &mut vars, &mut ctx, &[], 2_000).await;

        assert!(
            matches!(report.outcome, ReportOutcome::Success),
            "{}",
            format_step_toon(&report)
        );
        assert_eq!(recovery.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(taps(&driver), 1, "a tap that landed SHALL not run twice");
    }

    #[tokio::test]
    async fn without_a_recovery_hook_a_companion_death_fails_the_step() {
        let driver = screen();
        driver.set_error_on_calls(
            "tap",
            golem_events::FailureCode::DeviceCompanionUnreachable,
            "connection refused",
            &[1],
        );
        let mut vars = make_vars();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut ctx = crate::context::test_ctx(tmp.path());

        let report =
            execute_single_step(&tap("Sign in"), &driver, &mut vars, &mut ctx, &[], 2_000).await;

        assert!(
            matches!(
                report.outcome,
                ReportOutcome::Failed {
                    code: golem_events::FailureCode::DeviceCompanionUnreachable,
                    ..
                }
            ),
            "{}",
            format_step_toon(&report)
        );
    }
}
