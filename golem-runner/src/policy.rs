#[cfg(test)]
use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;
use golem_vars::VariableStore;

use crate::actions::execute_action;
use crate::capture::capture_failure_screenshot;
use crate::context::ExecutionContext;

/// Result of executing a step with policy
#[derive(Debug, PartialEq, Eq)]
pub enum StepOutcome {
    /// Step completed successfully (possibly after retries)
    Success,
    /// if_fail = "warn": step failed but execution continues
    Warning(String),
    /// if_fail = "ignore": step failed, silently continue
    Ignored,
}

/// Default retry delay in milliseconds
const DEFAULT_RETRY_DELAY_MS: u64 = 1_000;

/// Execute a step with if_fail, timeout, and retry policies applied.
///
/// This wraps [`execute_action`] with:
/// - **timeout**: cancels the step if it exceeds the configured duration
/// - **retry**: retries the step up to N times on failure
/// - **if_fail**: controls what happens when all attempts fail
pub async fn execute_step_with_policy(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    default_timeout_ms: u64,
    ctx: &ExecutionContext<'_>,
    apps: &[golem_parser::AppConfig],
) -> Result<StepOutcome> {
    let timeout_ms = step.timeout.unwrap_or(default_timeout_ms);
    let max_retries = step.retry.unwrap_or(0);
    let retry_delay_ms = step.retry_delay.unwrap_or(DEFAULT_RETRY_DELAY_MS);
    let if_fail = step.if_fail.as_deref().unwrap_or("error");

    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(retry_delay_ms)).await;
        }

        match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            execute_action(step, driver, vars, ctx, apps),
        )
        .await
        {
            Ok(Ok(())) => return Ok(StepOutcome::Success),
            Ok(Err(e)) => {
                last_error = Some(e);
                continue;
            }
            Err(_elapsed) => {
                last_error =
                    Some(anyhow::anyhow!("Step timed out after {}ms", timeout_ms));
                continue;
            }
        }
    }

    // All attempts exhausted — capture screenshot before applying if_fail policy.
    // Only capture for "error" and "warn" policies, not "ignore".
    if if_fail != "ignore" {
        let _ = capture_failure_screenshot(
            driver,
            ctx.capture_config,
            ctx.flow_name,
            ctx.block_name.unwrap_or("unnamed"),
            ctx.step_index,
            if_fail,
        )
        .await;
    }

    let error =
        last_error.unwrap_or_else(|| anyhow::anyhow!("step failed with no error details"));
    match if_fail {
        "warn" => Ok(StepOutcome::Warning(error.to_string())),
        "ignore" => Ok(StepOutcome::Ignored),
        _ => Err(error), // "error" (default) — propagate
    }
}

/// Apply if_fail, timeout, and retry policies around an arbitrary async executor.
///
/// This is the testable core: the `executor` closure receives the step and
/// returns a future that resolves to `Result<()>`.
#[cfg(test)]
async fn apply_policy<F, Fut>(
    step: &Step,
    default_timeout_ms: u64,
    executor: F,
) -> Result<StepOutcome>
where
    F: Fn(&Step) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let timeout_ms = step.timeout.unwrap_or(default_timeout_ms);
    let max_retries = step.retry.unwrap_or(0);
    let retry_delay_ms = step.retry_delay.unwrap_or(DEFAULT_RETRY_DELAY_MS);
    let if_fail = step.if_fail.as_deref().unwrap_or("error");

    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(retry_delay_ms)).await;
        }

        match tokio::time::timeout(Duration::from_millis(timeout_ms), executor(step)).await {
            Ok(Ok(())) => return Ok(StepOutcome::Success),
            Ok(Err(e)) => {
                last_error = Some(e);
                continue;
            }
            Err(_elapsed) => {
                last_error =
                    Some(anyhow::anyhow!("Step timed out after {}ms", timeout_ms));
                continue;
            }
        }
    }

    let error =
        last_error.unwrap_or_else(|| anyhow::anyhow!("step failed with no error details"));
    match if_fail {
        "warn" => Ok(StepOutcome::Warning(error.to_string())),
        "ignore" => Ok(StepOutcome::Ignored),
        _ => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    const DEFAULT_TIMEOUT_MS: u64 = 10_000;

    /// Helper: build a minimal Step with all optional fields set to None/defaults.
    fn make_step() -> Step {
        Step {
            action: "tap".to_string(),
            on_text: Some("OK".to_string()),
            on_accessibility_label: None,
            on_index: None,
            on_enabled: None,
            on_checked: None,
            on_clickable: None,
            on_below: None,
            on_above: None,
            on_right_of: None,
            on_left_of: None,
            on: None,
            input: None,
            if_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            auto_scroll: None,
            max_scrolls: None,
            scroll_timeout: None,
            within: None,
            params: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------
    // 1. Step succeeds -> StepOutcome::Success
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn step_succeeds_returns_success() {
        let step = make_step();
        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, |_| async { Ok(()) }).await;

        assert!(result.is_ok());
        assert_eq!(result.expect("should be ok"), StepOutcome::Success);
    }

    // -----------------------------------------------------------------
    // 2. Step fails with if_fail="error" -> returns Err
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn step_fails_with_on_fail_error_returns_err() {
        let mut step = make_step();
        step.if_fail = Some("error".to_string());

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, |_| async {
            Err(anyhow::anyhow!("element not found"))
        })
        .await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be err").to_string();
        assert!(
            err_msg.contains("element not found"),
            "error should propagate: {err_msg}"
        );
    }

    // -----------------------------------------------------------------
    // 3. Step fails with if_fail="warn" -> StepOutcome::Warning
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn step_fails_with_on_fail_warn_returns_warning() {
        let mut step = make_step();
        step.if_fail = Some("warn".to_string());

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, |_| async {
            Err(anyhow::anyhow!("something went wrong"))
        })
        .await;

        assert!(result.is_ok());
        match result.expect("should be ok") {
            StepOutcome::Warning(msg) => {
                assert!(
                    msg.contains("something went wrong"),
                    "warning message should contain error: {msg}"
                );
            }
            other => panic!("expected Warning, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // 4. Step fails with if_fail="ignore" -> StepOutcome::Ignored
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn step_fails_with_on_fail_ignore_returns_ignored() {
        let mut step = make_step();
        step.if_fail = Some("ignore".to_string());

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, |_| async {
            Err(anyhow::anyhow!("ignored failure"))
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.expect("should be ok"), StepOutcome::Ignored);
    }

    // -----------------------------------------------------------------
    // 5. Step timeout triggers failure
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn step_timeout_triggers_failure() {
        let mut step = make_step();
        step.timeout = Some(50); // 50ms timeout
        step.if_fail = Some("error".to_string());

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, |_| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(())
        })
        .await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be err").to_string();
        assert!(
            err_msg.contains("timed out"),
            "error should mention timeout: {err_msg}"
        );
    }

    // -----------------------------------------------------------------
    // 6. Retry: step fails once then succeeds -> Success
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn retry_fails_once_then_succeeds() {
        let mut step = make_step();
        step.retry = Some(2);
        step.retry_delay = Some(10); // fast for tests

        let attempt_count = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&attempt_count);

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, move |_| {
            let count = Arc::clone(&count);
            async move {
                let attempt = count.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err(anyhow::anyhow!("first attempt fails"))
                } else {
                    Ok(())
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.expect("should be ok"), StepOutcome::Success);
        assert_eq!(
            attempt_count.load(Ordering::SeqCst),
            2,
            "should have attempted twice"
        );
    }

    // -----------------------------------------------------------------
    // 7. Retry: step fails all retries -> if_fail applies
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn retry_exhausted_applies_on_fail() {
        let mut step = make_step();
        step.retry = Some(2); // 3 total attempts (0, 1, 2)
        step.retry_delay = Some(10);
        step.if_fail = Some("warn".to_string());

        let attempt_count = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&attempt_count);

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, move |_| {
            let count = Arc::clone(&count);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("persistent failure"))
            }
        })
        .await;

        assert!(result.is_ok());
        match result.expect("should be ok") {
            StepOutcome::Warning(msg) => {
                assert!(msg.contains("persistent failure"));
            }
            other => panic!("expected Warning, got {other:?}"),
        }
        assert_eq!(
            attempt_count.load(Ordering::SeqCst),
            3,
            "should have attempted 3 times (initial + 2 retries)"
        );
    }

    // -----------------------------------------------------------------
    // 8. Default if_fail is "error"
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn default_on_fail_is_error() {
        let step = make_step(); // if_fail is None -> defaults to "error"

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, |_| async {
            Err(anyhow::anyhow!("default policy"))
        })
        .await;

        assert!(result.is_err(), "default if_fail SHALL propagate as Err");
    }

    // -----------------------------------------------------------------
    // 9. Default timeout is used when step has no timeout
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn default_timeout_is_used_when_step_has_no_timeout() {
        let step = make_step(); // timeout is None

        // Use a very short default timeout to verify it's applied
        let result = apply_policy(&step, 50, |_| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(())
        })
        .await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be err").to_string();
        assert!(
            err_msg.contains("timed out"),
            "should use default timeout: {err_msg}"
        );
    }

    // -----------------------------------------------------------------
    // 10. Timeout with retry — each attempt gets its own timeout
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn timeout_with_retry_each_attempt_has_own_timeout() {
        let mut step = make_step();
        step.timeout = Some(50);
        step.retry = Some(1); // 2 total attempts
        step.retry_delay = Some(10);
        step.if_fail = Some("warn".to_string());

        let attempt_count = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&attempt_count);

        let result = apply_policy(&step, DEFAULT_TIMEOUT_MS, move |_| {
            let count = Arc::clone(&count);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(())
            }
        })
        .await;

        assert!(result.is_ok());
        match result.expect("should be ok") {
            StepOutcome::Warning(msg) => {
                assert!(msg.contains("timed out"));
            }
            other => panic!("expected Warning, got {other:?}"),
        }
        assert_eq!(
            attempt_count.load(Ordering::SeqCst),
            2,
            "both attempts should have been tried"
        );
    }

    // -----------------------------------------------------------------
    // 11. Step-level timeout overrides default
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn step_timeout_overrides_default() {
        let mut step = make_step();
        step.timeout = Some(500); // generous step timeout

        // Default is very short, but step timeout should override
        let result = apply_policy(&step, 1, |_| async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(())
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.expect("should be ok"), StepOutcome::Success);
    }
}
