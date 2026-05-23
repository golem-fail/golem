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

/// Default base timeout in milliseconds (5 seconds).
pub const DEFAULT_BASE_TIMEOUT_MS: u64 = 5_000;

/// Compute the effective timeout for a step.
///
/// Priority:
/// 1. `step.timeout` (explicit per-step override) — always wins.
/// 2. `base_timeout_ms * action_multiplier`, raised to cover any intrinsic
///    gesture duration (+ 2s settle buffer).
///
/// The `base_timeout_ms` comes from `[flow.options] step_timeout` or the
/// default 5000ms.
pub fn effective_timeout(step: &Step, base_timeout_ms: u64) -> u64 {
    if let Some(t) = step.timeout {
        return t;
    }

    let multiplied = base_timeout_ms * action_multiplier(step);

    // If the step has an intrinsic duration (gesture, long_press, rotate),
    // ensure timeout covers it plus 2s for settle.
    let intrinsic = intrinsic_duration_ms(step);
    if intrinsic > 0 {
        multiplied.max(intrinsic + 2_000)
    } else {
        multiplied
    }
}

/// Per-action timeout multiplier applied to the base timeout.
fn action_multiplier(step: &Step) -> u64 {
    // `within = { ... }` on a scrolling step (either action="scroll"
    // or any step with auto_scroll=true) means the engine runs in
    // two phases: a page-level scroll to bring the container into
    // view, then an inner-scrollable scroll to find the target.
    // Each phase needs its own slice of the budget — auto-locate
    // alone can eat 10-15s on slower devices, leaving the inner
    // phase starved. Add 4x (=20s on the 5s base) when both apply.
    let within_bump = if step.within.is_some() { 4 } else { 0 };

    // auto_scroll forces 6x minimum regardless of action.
    if step.auto_scroll == Some(true) {
        return 6 + within_bump;
    }

    match step.action.as_str() {
        // 1x — instant actions and capture-only steps that don't go
        // through the element resolver / settle path.
        "screenshot" | "start_recording" | "stop_recording" | "add_media"
        | "fail" | "load_fixture" | "push_notification" | "set_variable"
        | "log" | "clear_data" | "press" | "rotate" | "dark_mode" | "set_location"
        | "grant_permission" | "revoke_permission" | "hide_keyboard" => 1,

        // 2x — interactions that include element resolution + post-action
        // settle (the first tap after a fresh app launch on iOS 26 spends
        // multiple seconds on WebKit Inspector enrichment + tree
        // stabilisation; a 1x = 5s budget consistently underflows).
        "tap" | "doubleTap" | "backspace" | "long_press" | "swipe"
        | "pinch" | "gesture"
        | "type" | "assert_visible" | "assert_checked" | "assert_not_visible"
        | "assert_alert" | "accept_alert" | "dismiss_alert"
        | "read" => 2,

        // 3x — app lifecycle (cold start)
        "launch" | "stop" => 3,

        // 4x — external scripts (unknown duration)
        "bash" | "run" => 4,

        // 6x — scroll (the within_bump above bumps to 10x when a
        // container is set), network I/O.
        "scroll" => 6 + within_bump,
        "http_get" | "http_post" | "http_put" | "http_patch"
        | "http_delete" | "open_link" => 6,

        // 48x — email polling (240s at 5s base)
        "await_email" => 48,

        // Unknown actions get 2x as safe default
        _ => 2,
    }
}

/// Intrinsic duration of a gesture step (ms).
///
/// Returns 0 for non-gesture actions. Used to ensure the timeout
/// covers the gesture itself plus settle time.
fn intrinsic_duration_ms(step: &Step) -> u64 {
    match step.action.as_str() {
        "long_press" => {
            step.params.get("duration")
                .and_then(|v| v.as_integer())
                .map(|d| d.max(0) as u64)
                .unwrap_or(1_000)
        }
        "swipe" => {
            // 3+ point swipe uses duration param; 2-point is instant
            let point_count = step.points.len()
                + step.start.as_ref().map_or(0, |_| 1)
                + step.end.as_ref().map_or(0, |_| 1);
            if point_count >= 3 {
                step.duration.unwrap_or(300)
            } else {
                0
            }
        }
        "gesture" => {
            let duration_per_segment = step.duration.unwrap_or(300);
            let max_points = step.fingers.iter()
                .map(|f| f.points.len())
                .max()
                .unwrap_or(2);
            duration_per_segment * max_points.saturating_sub(1) as u64
        }
        "rotate" => {
            if let Some(degrees) = step.rotation {
                let velocity = step.velocity.unwrap_or(180.0);
                ((degrees.abs() / velocity) * 1000.0) as u64
            } else {
                0
            }
        }
        "type" => {
            // ~200ms per character (iOS WebView is slow through JS bridge)
            let char_count = step.input.as_deref()
                .or(step.on_text.as_deref())
                .map(|s| s.len())
                .unwrap_or(0);
            (char_count as u64) * 200
        }
        "backspace" => {
            let count = step.params.get("count")
                .and_then(|v| v.as_integer())
                .map(|n| n.max(0) as u64)
                .unwrap_or(1);
            count * 200
        }
        _ => 0,
    }
}

/// Does the post-action settle wait apply to this action?
///
/// Mutating actions (tap, type, swipe, etc.) need the UI to stop
/// reacting before the next step starts asserting / finding /
/// interacting. Read-only actions (assert_*, read, screenshot)
/// don't. The settle wall-clock runs *outside* the next step's
/// timeout — that way a user-set `timeout = 5000` on an assert
/// reflects "find this within 5s" rather than "find this within
/// 5s minus whatever the previous tap's UI is still doing."
fn needs_post_settle(step: &Step) -> bool {
    matches!(
        step.action.as_str(),
        "tap"
            | "double_tap"
            | "type"
            | "backspace"
            | "long_press"
            | "swipe"
            | "scroll"
            | "hide_keyboard"
            | "pinch"
            | "rotate"
            | "gesture"
            | "press"
            | "launch"
            | "stop"
            | "accept_alert"
            | "dismiss_alert"
            | "dark_mode"
            | "set_location"
            | "open_link"
    )
}

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
    base_timeout_ms: u64,
    ctx: &ExecutionContext<'_>,
    apps: &[golem_parser::AppConfig],
) -> Result<StepOutcome> {
    let timeout_ms = effective_timeout(step, base_timeout_ms);
    let max_retries = step.retry.unwrap_or(0);
    let retry_delay_ms = step.retry_delay.unwrap_or(DEFAULT_RETRY_DELAY_MS);
    let if_fail = step.if_fail.as_deref().unwrap_or("error");

    // Sub the outer step deadline back into the companion HTTP client so
    // a wedged companion fails fast at the connection layer (clean
    // network error) instead of cascading the full step budget into
    // every later request. 500ms headroom lets reqwest fire before
    // the outer `tokio::time::timeout` cancels.
    let request_timeout_ms = timeout_ms.saturating_sub(500).max(1_000);
    driver.set_request_timeout(Duration::from_millis(request_timeout_ms));

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
            Ok(Ok(())) => {
                // Post-action settle runs out-of-band: its wall-clock
                // doesn't consume this step's budget nor the next
                // step's. Action handlers used to call wait_for_settle
                // inline, which counted toward whichever step the
                // animation was finishing — a stressed UI then made
                // user-budgeted timeouts on later steps unreachable.
                if needs_post_settle(step) {
                    let started = std::time::Instant::now();
                    let _ = crate::resolution::wait_for_settle(driver).await;
                    let elapsed = started.elapsed();
                    // `wait_for_settle`'s internal SETTLE_TIMEOUT is
                    // 1500ms — anything close to that means it gave up
                    // waiting for the UI to stop animating.
                    let stable = elapsed < std::time::Duration::from_millis(1300);
                    ctx.substep(golem_events::SubstepEvent::PostSettle {
                        action: step.action.clone(),
                        duration_ms: elapsed.as_millis() as u64,
                        stable,
                    });
                }
                return Ok(StepOutcome::Success);
            }
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
            ctx.block_name.unwrap_or("unnamed"),
            ctx.global_step_index,
            ctx.block_iteration,
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
    base_timeout_ms: u64,
    executor: F,
) -> Result<StepOutcome>
where
    F: Fn(&Step) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let timeout_ms = effective_timeout(step, base_timeout_ms);
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    const DEFAULT_TIMEOUT_MS: u64 = 10_000;

    /// Helper: build a minimal Step with all optional fields set to None/defaults.
    fn make_step() -> Step {
        Step {
            action: "tap".to_string(),
            on_text: Some("OK".to_string()),
            ..Default::default()
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

    // -----------------------------------------------------------------
    // 12. Action multiplier: tap = 2x, assert_visible = 2x, scroll = 6x
    // -----------------------------------------------------------------
    #[test]
    fn action_multiplier_values() {
        let tap = Step { action: "tap".into(), ..Default::default() };
        assert_eq!(effective_timeout(&tap, 5_000), 10_000);

        let assert_vis = Step { action: "assert_visible".into(), ..Default::default() };
        assert_eq!(effective_timeout(&assert_vis, 5_000), 10_000);

        let scroll = Step { action: "scroll".into(), ..Default::default() };
        assert_eq!(effective_timeout(&scroll, 5_000), 30_000);

        let launch = Step { action: "launch".into(), ..Default::default() };
        assert_eq!(effective_timeout(&launch, 5_000), 15_000);

        let bash = Step { action: "bash".into(), ..Default::default() };
        assert_eq!(effective_timeout(&bash, 5_000), 20_000);

        let email = Step { action: "await_email".into(), ..Default::default() };
        assert_eq!(effective_timeout(&email, 5_000), 240_000);
    }

    // -----------------------------------------------------------------
    // 13. auto_scroll forces 6x minimum
    // -----------------------------------------------------------------
    #[test]
    fn auto_scroll_forces_6x() {
        let mut step = Step { action: "tap".into(), ..Default::default() };
        step.auto_scroll = Some(true);
        // tap is normally 1x (5s), but auto_scroll forces 6x (30s)
        assert_eq!(effective_timeout(&step, 5_000), 30_000);
    }

    // -----------------------------------------------------------------
    // 13a. `within` bumps scrolling timeouts by 4x
    // -----------------------------------------------------------------
    #[test]
    fn within_adds_to_scroll_timeout() {
        let within = golem_parser::SelectorGroup { text: Some("Carousel".into()), ..Default::default() };

        // scroll without within stays at 6x = 30s.
        let scroll = Step { action: "scroll".into(), ..Default::default() };
        assert_eq!(effective_timeout(&scroll, 5_000), 30_000);

        // scroll with within bumps to 10x = 50s.
        let scroll_within = Step { action: "scroll".into(), within: Some(within.clone()), ..Default::default() };
        assert_eq!(effective_timeout(&scroll_within, 5_000), 50_000);

        // auto_scroll alone stays at 6x = 30s.
        let auto = Step { action: "tap".into(), auto_scroll: Some(true), ..Default::default() };
        assert_eq!(effective_timeout(&auto, 5_000), 30_000);

        // auto_scroll + within bumps to 10x = 50s.
        let auto_within = Step { action: "tap".into(), auto_scroll: Some(true), within: Some(within), ..Default::default() };
        assert_eq!(effective_timeout(&auto_within, 5_000), 50_000);
    }

    // -----------------------------------------------------------------
    // 14. Intrinsic duration: long_press extends timeout
    // -----------------------------------------------------------------
    #[test]
    fn long_press_duration_extends_timeout() {
        let mut step = Step { action: "long_press".into(), ..Default::default() };
        // Default long_press duration=1000, intrinsic=1000, floor=3000.
        // Multiplied=10000 (2x). max(10000, 3000) = 10000.
        assert_eq!(effective_timeout(&step, 5_000), 10_000);

        // long_press with duration=8000: intrinsic=8000, floor=10000,
        // multiplied=10000. max(10000, 10000) = 10000.
        step.params.insert("duration".to_string(), toml::Value::Integer(8_000));
        assert_eq!(effective_timeout(&step, 5_000), 10_000);
    }

    // -----------------------------------------------------------------
    // 15. Intrinsic duration: slow rotate extends timeout
    // -----------------------------------------------------------------
    #[test]
    fn slow_rotate_extends_timeout() {
        let mut step = Step { action: "rotate".into(), ..Default::default() };
        step.rotation = Some(720.0);
        step.velocity = Some(45.0);
        // 720/45 * 1000 = 16000ms intrinsic. floor = 18000
        // Multiplied = 5000 (1x). max(5000, 18000) = 18000
        assert_eq!(effective_timeout(&step, 5_000), 18_000);
    }

    // -----------------------------------------------------------------
    // 16. Per-step timeout always wins over multiplier
    // -----------------------------------------------------------------
    #[test]
    fn step_timeout_overrides_multiplier() {
        let mut step = Step { action: "await_email".into(), ..Default::default() };
        step.timeout = Some(500);
        // await_email is 48x but step.timeout=500 wins
        assert_eq!(effective_timeout(&step, 5_000), 500);
    }

    // -----------------------------------------------------------------
    // 17. Unknown action gets 2x default
    // -----------------------------------------------------------------
    #[test]
    fn unknown_action_gets_2x() {
        let step = Step { action: "future_action".into(), ..Default::default() };
        assert_eq!(effective_timeout(&step, 5_000), 10_000);
    }

    // -----------------------------------------------------------------
    // 18. Type scales with input length
    // -----------------------------------------------------------------
    #[test]
    fn type_scales_with_input_length() {
        // Short input: 5 chars * 200ms = 1000ms intrinsic, under 2x (10000), no effect
        let mut step = Step { action: "type".into(), ..Default::default() };
        step.input = Some("hello".to_string());
        assert_eq!(effective_timeout(&step, 5_000), 10_000); // 2x base

        // Long input: 80 chars * 200ms = 16000ms intrinsic + 2s = 18000, exceeds 2x (10000)
        step.input = Some("ab".repeat(40));
        assert_eq!(effective_timeout(&step, 5_000), 18_000);
    }
}
