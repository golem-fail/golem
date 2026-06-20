// golem-report: test reporting
#![deny(clippy::unwrap_used)]

pub mod accumulator;
pub mod human;
pub mod json;
pub mod junit;
pub mod output;
pub mod stream;
pub mod toon;
pub mod flake;

use serde::Serialize;

/// Serializable substep detail for report output.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubstepDetail {
    ElementResolved {
        selector: String,
        bounds: golem_events::Rect,
        tap_point: golem_events::Point,
    },
    ElementNotFound {
        selector: String,
        timeout_ms: u64,
    },
    Tap {
        point: golem_events::Point,
        #[serde(skip_serializing_if = "Option::is_none")]
        element_bounds: Option<golem_events::Rect>,
    },
    DoubleTap {
        point: golem_events::Point,
        #[serde(skip_serializing_if = "Option::is_none")]
        element_bounds: Option<golem_events::Rect>,
    },
    LongPress {
        point: golem_events::Point,
        duration_ms: u64,
    },
    TextInput {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        field_bounds: Option<golem_events::Rect>,
    },
    Backspace {
        count: u32,
    },
    Swipe {
        from: golem_events::Point,
        to: golem_events::Point,
    },
    ScrollStarted {
        selector: String,
        direction: String,
    },
    ScrollAttempt {
        attempt: u32,
        direction: String,
        strategy_index: usize,
        container: bool,
        from: golem_events::Point,
        to: golem_events::Point,
        result: String,
        tree_stats: golem_events::TreeStats,
    },
    ScrollFound {
        selector: String,
        position: golem_events::Point,
        total_attempts: u32,
    },
    ScrollDirectionReversed {
        to_direction: String,
        reason: String,
    },
    ScrollStrategySwitch {
        to_index: usize,
        reason: String,
    },
    AssertionMatch {
        expected: String,
        actual: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        element_bounds: Option<golem_events::Rect>,
    },
    AssertionMismatch {
        expected: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        actual: Option<String>,
    },
    RetryAttempt {
        attempt: u32,
        max: u32,
        delay_ms: u64,
        error: String,
    },
    HttpRequest {
        method: String,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
        duration_ms: u64,
    },
    BashCommand {
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        duration_ms: u64,
    },
    AppLaunch {
        bundle: String,
        duration_ms: u64,
    },
    AppStop {
        bundle: String,
    },
    PostSettle {
        action: String,
        duration_ms: u64,
        stable: bool,
    },
    DriverWarning {
        message: String,
    },
    Screenshot {
        path: String,
    },
    BarrierAborted {
        step_count: u64,
    },
}

/// Convert a SubstepEvent from the event stream to a serializable SubstepDetail.
impl From<&golem_events::SubstepEvent> for SubstepDetail {
    fn from(event: &golem_events::SubstepEvent) -> Self {
        match event {
            golem_events::SubstepEvent::ElementResolved { selector, bounds, tap_point } =>
                SubstepDetail::ElementResolved { selector: selector.clone(), bounds: *bounds, tap_point: *tap_point },
            golem_events::SubstepEvent::ElementNotFound { selector, timeout_ms } =>
                SubstepDetail::ElementNotFound { selector: selector.clone(), timeout_ms: *timeout_ms },
            golem_events::SubstepEvent::Tap { point, element_bounds } =>
                SubstepDetail::Tap { point: *point, element_bounds: *element_bounds },
            golem_events::SubstepEvent::DoubleTap { point, element_bounds } =>
                SubstepDetail::DoubleTap { point: *point, element_bounds: *element_bounds },
            golem_events::SubstepEvent::LongPress { point, duration_ms, element_bounds: _ } =>
                SubstepDetail::LongPress { point: *point, duration_ms: *duration_ms },
            golem_events::SubstepEvent::TextInput { text, field_bounds } =>
                SubstepDetail::TextInput { text: text.clone(), field_bounds: *field_bounds },
            golem_events::SubstepEvent::Backspace { count } =>
                SubstepDetail::Backspace { count: *count },
            golem_events::SubstepEvent::Swipe { from, to, duration_ms: _ } =>
                SubstepDetail::Swipe { from: *from, to: *to },
            golem_events::SubstepEvent::ScrollStarted { selector, direction } =>
                SubstepDetail::ScrollStarted { selector: selector.clone(), direction: direction.clone() },
            golem_events::SubstepEvent::ScrollAttempt { attempt, direction, strategy_index, container, from, to, result, tree_stats } =>
                SubstepDetail::ScrollAttempt {
                    attempt: *attempt, direction: direction.clone(), strategy_index: *strategy_index,
                    container: *container,
                    from: *from, to: *to, result: format!("{result:?}"),
                    tree_stats: *tree_stats,
                },
            golem_events::SubstepEvent::ScrollFound { selector, position, total_attempts } =>
                SubstepDetail::ScrollFound { selector: selector.clone(), position: *position, total_attempts: *total_attempts },
            golem_events::SubstepEvent::ScrollDirectionReversed { to_direction, reason } =>
                SubstepDetail::ScrollDirectionReversed { to_direction: to_direction.clone(), reason: reason.clone() },
            golem_events::SubstepEvent::ScrollStrategySwitch { to_index, reason } =>
                SubstepDetail::ScrollStrategySwitch { to_index: *to_index, reason: reason.clone() },
            golem_events::SubstepEvent::AssertionMatch { expected, actual, element_bounds } =>
                SubstepDetail::AssertionMatch { expected: expected.clone(), actual: actual.clone(), element_bounds: *element_bounds },
            golem_events::SubstepEvent::AssertionMismatch { expected, actual } =>
                SubstepDetail::AssertionMismatch { expected: expected.clone(), actual: actual.clone() },
            golem_events::SubstepEvent::RetryAttempt { attempt, max, delay_ms, error } =>
                SubstepDetail::RetryAttempt { attempt: *attempt, max: *max, delay_ms: *delay_ms, error: error.clone() },
            golem_events::SubstepEvent::HttpRequest { method, url, status, duration_ms } =>
                SubstepDetail::HttpRequest { method: method.clone(), url: url.clone(), status: *status, duration_ms: *duration_ms },
            golem_events::SubstepEvent::BashCommand { command, exit_code, duration_ms } =>
                SubstepDetail::BashCommand { command: command.clone(), exit_code: *exit_code, duration_ms: *duration_ms },
            golem_events::SubstepEvent::AppLaunch { bundle, duration_ms } =>
                SubstepDetail::AppLaunch { bundle: bundle.clone(), duration_ms: *duration_ms },
            golem_events::SubstepEvent::AppStop { bundle } =>
                SubstepDetail::AppStop { bundle: bundle.clone() },
            golem_events::SubstepEvent::PostSettle { action, duration_ms, stable } =>
                SubstepDetail::PostSettle { action: action.clone(), duration_ms: *duration_ms, stable: *stable },
            golem_events::SubstepEvent::DriverWarning { message } =>
                SubstepDetail::DriverWarning { message: message.clone() },
            golem_events::SubstepEvent::Screenshot { path } =>
                SubstepDetail::Screenshot { path: path.clone() },
            golem_events::SubstepEvent::BarrierAborted { step_count } =>
                SubstepDetail::BarrierAborted { step_count: *step_count },
            golem_events::SubstepEvent::AlertFound { text } =>
                SubstepDetail::AssertionMatch {
                    expected: "alert".to_string(),
                    actual: text.clone().unwrap_or_else(|| "alert present".to_string()),
                    element_bounds: None,
                },
        }
    }
}

/// Result of a single step within a flow.
#[derive(Clone)]
pub struct StepReport {
    /// Global step index across all blocks.
    pub global_step_index: u64,
    /// Name of the block containing this step.
    pub block_name: String,
    /// Iteration of the containing block (0 for single-pass blocks).
    pub block_iteration: u32,
    /// Index within the block.
    pub step_index_in_block: usize,
    /// The action performed (e.g. "tap", "type", "assert_visible").
    pub action: String,
    /// The target element text or identifier.
    pub target: String,
    /// The outcome of this step.
    pub outcome: StepOutcome,
    /// How long this step took, in milliseconds.
    pub duration_ms: u64,
    /// Number of retry attempts.
    pub retry_count: u32,
    /// Path to screenshot if captured.
    pub screenshot_path: Option<String>,
    /// Detailed substep events.
    pub substeps: Vec<SubstepDetail>,
    /// Tree fetch statistics for this step.
    pub tree_stats: golem_events::TreeStats,
    /// ISO-8601 UTC wall-clock when the step started, if the report was
    /// built from a live event stream (None if synthesized directly).
    pub started_at: Option<String>,
    /// ISO-8601 UTC wall-clock when the step finished.
    pub finished_at: Option<String>,
}

/// Possible outcomes for a single step.
#[derive(Clone)]
pub enum StepOutcome {
    /// Step completed successfully.
    Success,
    /// Step completed with a warning.
    Warning { message: String, code: golem_events::FailureCode },
    /// Step failed with an error message.
    Failed { message: String, code: golem_events::FailureCode },
    /// Step was skipped.
    Skipped,
}

/// A single performance snapshot captured at a block boundary.
#[derive(Debug, Clone)]
pub struct PerfSnapshot {
    /// Label: `{block_name}:{device_name}:{iteration}` or `block_N({action}:{target}):...`
    pub label: String,
    /// Resident memory in MB (PSS on Android, RSS on iOS).
    pub memory_mb: Option<f64>,
    /// CPU usage percentage.
    pub cpu_percent: Option<f64>,
    /// Thread count.
    pub threads: Option<u32>,
    /// Open file descriptor count.
    pub file_descriptors: Option<u32>,
    /// App container size on disk in MB.
    pub disk_mb: Option<f64>,
    /// Cumulative network bytes received in KB.
    pub net_rx_kb: Option<f64>,
    /// Cumulative network bytes sent in KB.
    pub net_tx_kb: Option<f64>,
    /// Last app launch duration in milliseconds.
    pub launch_ms: Option<u64>,
    /// ISO 8601 capture timestamp.
    pub timestamp: String,
}

/// Result of a complete test flow.
#[derive(Default, Clone)]
pub struct FlowReport {
    /// Name of the flow (e.g. "login_flow").
    pub flow_name: String,
    /// Whether the flow passed overall.
    pub success: bool,
    /// When true, this flow was skipped (e.g. due to prior install failure).
    /// A skipped flow has `success = false` and `skipped_reason` set.
    pub skipped_reason: Option<String>,
    /// Individual step results, in order.
    pub step_results: Vec<StepReport>,
    /// Any flow-level warnings.
    pub warnings: Vec<String>,
    /// Total duration of the flow in milliseconds.
    pub duration_ms: u64,
    /// Random seed used, if applicable.
    pub seed: Option<u64>,
    /// Path to an error screenshot, if one was captured.
    pub screenshot_path: Option<String>,
    /// Name of the device the flow ran on.
    pub device_name: Option<String>,
    /// OS major version of the device (e.g. 18, 26, 34). Populated from
    /// FlowStarted/FlowFinished events; None when the report is synthesized
    /// without a live event stream (e.g. parse-failure reports).
    pub os_major: Option<u32>,
    /// Performance snapshots captured at block boundaries.
    pub perf_snapshots: Vec<PerfSnapshot>,
    /// ISO-8601 UTC wall-clock when the flow started.
    pub started_at: Option<String>,
    /// ISO-8601 UTC wall-clock when the flow finished.
    pub finished_at: Option<String>,
    /// Human-readable list of coverage axes this run ticked, derived
    /// from the FlowRun's slot. Populated when the run came from a
    /// non-trivial tick-box expansion. Examples: `["ios", "v26",
    /// "tablet"]`. Renderers surface this so users see which axes a
    /// Min/Smart run actually covered.
    pub covered_axes: Vec<String>,
    /// Screen recordings saved during this flow, one entry per
    /// recorded block iteration.
    pub recordings: Vec<RecordingEntry>,
    /// `--repeat` context. `None` for single-run suites; `Some(...)`
    /// when the suite was fanned out across multiple runs. Renderers
    /// and the flake-summary tally use this to partition flows by run.
    pub repeat: Option<golem_events::RepeatContext>,
    /// Failure code of the first failed/warned step, surfaced on the
    /// flow-level FAIL line. `None` for passing flows.
    pub first_failure_code: Option<golem_events::FailureCode>,
}

/// One screen recording produced by a recorded block iteration.
#[derive(Debug, Clone)]
pub struct RecordingEntry {
    pub block: String,
    pub iteration: u32,
    pub path: String,
}

impl FlowReport {
    /// Flow was deliberately spared by a coverage-group gate — a peer
    /// run in a `coverage = "one"` group already met the goal. These
    /// runs carry `success = true` (no CI failure) + a skip reason.
    ///
    /// Install-precondition skips (missing bundle, failed install
    /// script) are **not** `is_skipped` — they keep `success = false`
    /// and classify as [`is_failed`], since a broken install should
    /// still fail the suite.
    pub fn is_skipped(&self) -> bool {
        self.success && self.skipped_reason.is_some()
    }

    /// Flow ran and passed. Excludes coverage-group skips whose
    /// `success = true` is bookkeeping, not a real pass.
    pub fn is_passed(&self) -> bool {
        self.success && self.skipped_reason.is_none()
    }

    /// Flow did not succeed. Covers both real test failures and
    /// install-precondition failures (success=false + skipped_reason).
    /// Drives the suite's exit code.
    pub fn is_failed(&self) -> bool {
        !self.success
    }
}

/// Install script result (per `(device, bundle)` across the whole suite).
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub app_name: String,
    pub bundle_id: String,
    pub device_name: String,
    /// OS major version of the device (e.g. 18, 26, 34).
    pub os_major: Option<u32>,
    pub success: bool,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    /// Failure code when the install failed (A-domain). `None` on success.
    pub code: Option<golem_events::FailureCode>,
    /// ISO-8601 UTC wall-clock when the install started.
    pub started_at: Option<String>,
    /// ISO-8601 UTC wall-clock when the install finished.
    pub finished_at: Option<String>,
}

/// Result of an entire test suite (multiple flows).
#[derive(Default, Clone)]
pub struct SuiteReport {
    /// Individual flow results.
    pub flows: Vec<FlowReport>,
    /// Install script results (one per `(device, bundle)` pair attempted).
    pub installs: Vec<InstallReport>,
    /// Total wall-clock duration in milliseconds.
    pub total_duration_ms: u64,
    /// ISO-8601 UTC wall-clock when the suite started (first observed event).
    pub started_at: Option<String>,
    /// ISO-8601 UTC wall-clock when the suite finished.
    pub finished_at: Option<String>,
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use golem_events::{Point, Rect, SubstepEvent};

    fn flow_with(success: bool, skipped_reason: Option<String>) -> FlowReport {
        FlowReport {
            first_failure_code: None,
            flow_name: "f".into(),
            success,
            step_results: Vec::new(),
            warnings: Vec::new(),
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: Vec::new(),
            skipped_reason,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn flow_status_passed_when_success_and_no_skip_reason() {
        let f = flow_with(true, None);
        assert!(f.is_passed());
        assert!(!f.is_failed());
        assert!(!f.is_skipped());
    }

    #[test]
    fn flow_status_failed_when_not_success_and_no_skip_reason() {
        let f = flow_with(false, None);
        assert!(!f.is_passed());
        assert!(f.is_failed());
        assert!(!f.is_skipped());
    }

    #[test]
    fn flow_status_skipped_takes_priority_over_success() {
        // Coverage-group reclassify: success=true + skipped_reason=Some.
        // SHALL count as skipped, NOT as passed.
        let f = flow_with(true, Some("coverage group satisfied".into()));
        assert!(!f.is_passed());
        assert!(!f.is_failed());
        assert!(f.is_skipped());
    }

    #[test]
    fn flow_install_skip_classifies_as_failed_not_skipped() {
        // Pre-existing FlowSkipped convention for install-precondition
        // failures: success=false + reason=Some. These SHALL fail the
        // suite (is_failed = true) and NOT appear as a skipped run —
        // install failures are real problems even though the test was
        // never attempted.
        let f = flow_with(false, Some("install failed".into()));
        assert!(!f.is_passed());
        assert!(f.is_failed());
        assert!(!f.is_skipped());
    }

    #[test]
    fn tap_with_element_bounds_converts_correctly() {
        let event = SubstepEvent::Tap {
            point: Point { x: 150, y: 300 },
            element_bounds: Some(Rect { x: 100, y: 280, width: 100, height: 44 }),
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::Tap { point, element_bounds } => {
                assert_eq!(point.x, 150, "SHALL preserve tap point x");
                assert_eq!(point.y, 300, "SHALL preserve tap point y");
                let bounds = element_bounds.expect("SHALL preserve element_bounds");
                assert_eq!(bounds.x, 100);
                assert_eq!(bounds.y, 280);
                assert_eq!(bounds.width, 100);
                assert_eq!(bounds.height, 44);
            }
            other => panic!("SHALL produce Tap variant, got {other:?}"),
        }
    }

    #[test]
    fn element_resolved_preserves_bounds_and_tap_point() {
        let event = SubstepEvent::ElementResolved {
            selector: "text=Submit".into(),
            bounds: Rect { x: 20, y: 400, width: 200, height: 50 },
            tap_point: Point { x: 120, y: 425 },
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::ElementResolved { selector, bounds, tap_point } => {
                assert_eq!(selector, "text=Submit", "SHALL preserve selector");
                assert_eq!(bounds.x, 20, "SHALL preserve bounds.x");
                assert_eq!(bounds.width, 200, "SHALL preserve bounds.width");
                assert_eq!(tap_point.x, 120, "SHALL preserve tap_point.x");
                assert_eq!(tap_point.y, 425, "SHALL preserve tap_point.y");
            }
            other => panic!("SHALL produce ElementResolved variant, got {other:?}"),
        }
    }

    #[test]
    fn scroll_found_preserves_position_and_attempts() {
        let event = SubstepEvent::ScrollFound {
            selector: "text=Price".into(),
            position: Point { x: 200, y: 800 },
            total_attempts: 5,
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::ScrollFound { selector, position, total_attempts } => {
                assert_eq!(selector, "text=Price", "SHALL preserve selector");
                assert_eq!(position.x, 200, "SHALL preserve position.x");
                assert_eq!(position.y, 800, "SHALL preserve position.y");
                assert_eq!(total_attempts, 5, "SHALL preserve total_attempts");
            }
            other => panic!("SHALL produce ScrollFound variant, got {other:?}"),
        }
    }

    #[test]
    fn text_input_preserves_text() {
        let event = SubstepEvent::TextInput {
            text: "hello@example.com".into(),
            field_bounds: Some(Rect { x: 10, y: 100, width: 300, height: 40 }),
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::TextInput { text, field_bounds } => {
                assert_eq!(text, "hello@example.com", "SHALL preserve text");
                assert!(field_bounds.is_some(), "SHALL preserve field_bounds");
            }
            other => panic!("SHALL produce TextInput variant, got {other:?}"),
        }
    }

    #[test]
    fn app_launch_preserves_bundle_and_duration() {
        let event = SubstepEvent::AppLaunch {
            bundle: "com.example.app".into(),
            duration_ms: 1234,
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::AppLaunch { bundle, duration_ms } => {
                assert_eq!(bundle, "com.example.app", "SHALL preserve bundle");
                assert_eq!(duration_ms, 1234, "SHALL preserve duration_ms");
            }
            other => panic!("SHALL produce AppLaunch variant, got {other:?}"),
        }
    }

    #[test]
    fn element_not_found_preserves_timeout() {
        let event = SubstepEvent::ElementNotFound {
            selector: "text=Ghost".into(),
            timeout_ms: 10000,
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::ElementNotFound { selector, timeout_ms } => {
                assert_eq!(selector, "text=Ghost", "SHALL preserve selector");
                assert_eq!(timeout_ms, 10000, "SHALL preserve timeout_ms");
            }
            other => panic!("SHALL produce ElementNotFound variant, got {other:?}"),
        }
    }

    #[test]
    fn alert_found_with_text_maps_to_assertion_match() {
        let event = SubstepEvent::AlertFound {
            text: Some("Delete this item?".into()),
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::AssertionMatch { expected, actual, element_bounds } => {
                assert_eq!(expected, "alert", "SHALL set expected to 'alert'");
                assert_eq!(actual, "Delete this item?", "SHALL pass alert text as actual");
                assert!(element_bounds.is_none(), "SHALL set element_bounds to None");
            }
            other => panic!("SHALL produce AssertionMatch variant, got {other:?}"),
        }
    }

    #[test]
    fn alert_found_without_text_uses_default() {
        let event = SubstepEvent::AlertFound { text: None };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::AssertionMatch { actual, .. } => {
                assert_eq!(actual, "alert present",
                    "SHALL use 'alert present' as default when text is None");
            }
            other => panic!("SHALL produce AssertionMatch variant, got {other:?}"),
        }
    }

    #[test]
    fn tap_without_bounds_converts_with_none() {
        let event = SubstepEvent::Tap {
            point: Point { x: 50, y: 60 },
            element_bounds: None,
        };
        let detail = SubstepDetail::from(&event);
        match detail {
            SubstepDetail::Tap { point, element_bounds } => {
                assert_eq!(point.x, 50);
                assert_eq!(point.y, 60);
                assert!(element_bounds.is_none(), "SHALL preserve None bounds");
            }
            other => panic!("SHALL produce Tap variant, got {other:?}"),
        }
    }

    // 14. LongPress carries element_bounds on the event but the detail
    //     variant has no such field — conversion SHALL keep point and
    //     duration and silently drop the bounds.
    #[test]
    fn long_press_keeps_point_and_duration_dropping_bounds() {
        let event = SubstepEvent::LongPress {
            point: Point { x: 70, y: 90 },
            duration_ms: 800,
            element_bounds: Some(Rect { x: 1, y: 2, width: 3, height: 4 }),
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::LongPress { point, duration_ms } => {
                assert_eq!(point.x, 70, "SHALL preserve point.x");
                assert_eq!(point.y, 90, "SHALL preserve point.y");
                assert_eq!(duration_ms, 800, "SHALL preserve duration_ms");
            }
            other => panic!("SHALL produce LongPress variant, got {other:?}"),
        }
    }

    // 15. Swipe carries duration_ms on the event but the detail variant
    //     has only from/to — conversion SHALL keep endpoints and drop
    //     the duration.
    #[test]
    fn swipe_keeps_endpoints_dropping_duration() {
        let event = SubstepEvent::Swipe {
            from: Point { x: 10, y: 20 },
            to: Point { x: 30, y: 40 },
            duration_ms: Some(250),
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::Swipe { from, to } => {
                assert_eq!(from.x, 10, "SHALL preserve from.x");
                assert_eq!(from.y, 20, "SHALL preserve from.y");
                assert_eq!(to.x, 30, "SHALL preserve to.x");
                assert_eq!(to.y, 40, "SHALL preserve to.y");
            }
            other => panic!("SHALL produce Swipe variant, got {other:?}"),
        }
    }

    // 16. DoubleTap preserves point and element_bounds verbatim.
    #[test]
    fn double_tap_preserves_point_and_bounds() {
        let event = SubstepEvent::DoubleTap {
            point: Point { x: 5, y: 6 },
            element_bounds: None,
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::DoubleTap { point, element_bounds } => {
                assert_eq!(point.x, 5, "SHALL preserve point.x");
                assert_eq!(point.y, 6, "SHALL preserve point.y");
                assert!(element_bounds.is_none(), "SHALL preserve None bounds");
            }
            other => panic!("SHALL produce DoubleTap variant, got {other:?}"),
        }
    }

    // 17. ScrollAttempt formats the ScrollAttemptResult enum via Debug
    //     into the detail's `result` String, preserving the numeric
    //     fields, and copies the rest of the fields.
    #[test]
    fn scroll_attempt_debug_formats_result_and_copies_fields() {
        let event = SubstepEvent::ScrollAttempt {
            attempt: 3,
            direction: "down".into(),
            strategy_index: 2,
            container: false,
            from: Point { x: 100, y: 700 },
            to: Point { x: 100, y: 300 },
            result: golem_events::ScrollAttemptResult::Stall { count: 2, max: 3 },
            tree_stats: golem_events::TreeStats { fetches: 4, min_nodes: 10, max_nodes: 90 },
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::ScrollAttempt {
                attempt, direction, strategy_index, container, from, to, result, tree_stats,
            } => {
                assert_eq!(attempt, 3, "SHALL preserve attempt");
                assert_eq!(direction, "down", "SHALL preserve direction");
                assert_eq!(strategy_index, 2, "SHALL preserve strategy_index");
                assert!(!container, "SHALL preserve container flag");
                assert_eq!(from.y, 700, "SHALL preserve from.y");
                assert_eq!(to.y, 300, "SHALL preserve to.y");
                assert_eq!(result, "Stall { count: 2, max: 3 }",
                    "SHALL Debug-format the ScrollAttemptResult into result");
                assert_eq!(tree_stats.fetches, 4, "SHALL preserve tree_stats.fetches");
                assert_eq!(tree_stats.min_nodes, 10, "SHALL preserve tree_stats.min_nodes");
                assert_eq!(tree_stats.max_nodes, 90, "SHALL preserve tree_stats.max_nodes");
            }
            other => panic!("SHALL produce ScrollAttempt variant, got {other:?}"),
        }
    }

    // 18. ScrollAttempt with a fieldless result variant Debug-formats to
    //     the bare variant name.
    #[test]
    fn scroll_attempt_fieldless_result_debug_formats_to_name() {
        let event = SubstepEvent::ScrollAttempt {
            attempt: 1,
            direction: "up".into(),
            strategy_index: 0,
            container: false,
            from: Point { x: 0, y: 0 },
            to: Point { x: 0, y: 100 },
            result: golem_events::ScrollAttemptResult::BoundaryReached,
            tree_stats: golem_events::TreeStats::default(),
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::ScrollAttempt { result, .. } => {
                assert_eq!(result, "BoundaryReached",
                    "SHALL Debug-format a fieldless result to its variant name");
            }
            other => panic!("SHALL produce ScrollAttempt variant, got {other:?}"),
        }
    }

    // 19. AssertionMismatch carries an Option<String> actual; conversion
    //     SHALL pass through both Some and the absence of element_bounds
    //     (the variant has none).
    #[test]
    fn assertion_mismatch_preserves_optional_actual() {
        let some = SubstepEvent::AssertionMismatch {
            expected: "Welcome".into(),
            actual: Some("Goodbye".into()),
        };
        match SubstepDetail::from(&some) {
            SubstepDetail::AssertionMismatch { expected, actual } => {
                assert_eq!(expected, "Welcome", "SHALL preserve expected");
                assert_eq!(actual.as_deref(), Some("Goodbye"), "SHALL preserve Some actual");
            }
            other => panic!("SHALL produce AssertionMismatch variant, got {other:?}"),
        }
        let none = SubstepEvent::AssertionMismatch { expected: "X".into(), actual: None };
        match SubstepDetail::from(&none) {
            SubstepDetail::AssertionMismatch { actual, .. } => {
                assert!(actual.is_none(), "SHALL preserve None actual");
            }
            other => panic!("SHALL produce AssertionMismatch variant, got {other:?}"),
        }
    }

    // 20. RetryAttempt copies all numeric fields and clones the error.
    #[test]
    fn retry_attempt_preserves_all_fields() {
        let event = SubstepEvent::RetryAttempt {
            attempt: 2,
            max: 5,
            delay_ms: 1500,
            error: "timeout".into(),
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::RetryAttempt { attempt, max, delay_ms, error } => {
                assert_eq!(attempt, 2, "SHALL preserve attempt");
                assert_eq!(max, 5, "SHALL preserve max");
                assert_eq!(delay_ms, 1500, "SHALL preserve delay_ms");
                assert_eq!(error, "timeout", "SHALL preserve error");
            }
            other => panic!("SHALL produce RetryAttempt variant, got {other:?}"),
        }
    }

    // 21. HttpRequest preserves method/url/status/duration, including a
    //     None status.
    #[test]
    fn http_request_preserves_fields_with_none_status() {
        let event = SubstepEvent::HttpRequest {
            method: "POST".into(),
            url: "https://api.example.com/charge".into(),
            status: None,
            duration_ms: 42,
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::HttpRequest { method, url, status, duration_ms } => {
                assert_eq!(method, "POST", "SHALL preserve method");
                assert_eq!(url, "https://api.example.com/charge", "SHALL preserve url");
                assert!(status.is_none(), "SHALL preserve None status");
                assert_eq!(duration_ms, 42, "SHALL preserve duration_ms");
            }
            other => panic!("SHALL produce HttpRequest variant, got {other:?}"),
        }
    }

    // 22. BashCommand preserves command/exit_code/duration.
    #[test]
    fn bash_command_preserves_fields() {
        let event = SubstepEvent::BashCommand {
            command: "echo hi".into(),
            exit_code: Some(0),
            duration_ms: 7,
        };
        match SubstepDetail::from(&event) {
            SubstepDetail::BashCommand { command, exit_code, duration_ms } => {
                assert_eq!(command, "echo hi", "SHALL preserve command");
                assert_eq!(exit_code, Some(0), "SHALL preserve exit_code");
                assert_eq!(duration_ms, 7, "SHALL preserve duration_ms");
            }
            other => panic!("SHALL produce BashCommand variant, got {other:?}"),
        }
    }

    // 23. BarrierAborted preserves the step_count.
    #[test]
    fn barrier_aborted_preserves_step_count() {
        let event = SubstepEvent::BarrierAborted { step_count: 9 };
        match SubstepDetail::from(&event) {
            SubstepDetail::BarrierAborted { step_count } => {
                assert_eq!(step_count, 9, "SHALL preserve step_count");
            }
            other => panic!("SHALL produce BarrierAborted variant, got {other:?}"),
        }
    }

    // 24. The remaining string-carrying variants (ScrollStarted,
    //     ScrollDirectionReversed, ScrollStrategySwitch, AssertionMatch,
    //     PostSettle, AppStop, DriverWarning, Screenshot, Backspace) map
    //     1:1; spot-check representative fields and tags.
    #[test]
    fn string_and_scalar_variants_map_one_to_one() {
        match SubstepDetail::from(&SubstepEvent::ScrollStarted {
            selector: "text=Foo".into(), direction: "down".into(),
        }) {
            SubstepDetail::ScrollStarted { selector, direction } => {
                assert_eq!(selector, "text=Foo");
                assert_eq!(direction, "down");
            }
            other => panic!("SHALL produce ScrollStarted, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::ScrollDirectionReversed {
            to_direction: "up".into(), reason: "boundary".into(),
        }) {
            SubstepDetail::ScrollDirectionReversed { to_direction, reason } => {
                assert_eq!(to_direction, "up");
                assert_eq!(reason, "boundary");
            }
            other => panic!("SHALL produce ScrollDirectionReversed, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::ScrollStrategySwitch {
            to_index: 3, reason: "stall".into(),
        }) {
            SubstepDetail::ScrollStrategySwitch { to_index, reason } => {
                assert_eq!(to_index, 3);
                assert_eq!(reason, "stall");
            }
            other => panic!("SHALL produce ScrollStrategySwitch, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::AssertionMatch {
            expected: "A".into(), actual: "A".into(),
            element_bounds: Some(Rect { x: 1, y: 1, width: 1, height: 1 }),
        }) {
            SubstepDetail::AssertionMatch { expected, actual, element_bounds } => {
                assert_eq!(expected, "A");
                assert_eq!(actual, "A");
                assert!(element_bounds.is_some());
            }
            other => panic!("SHALL produce AssertionMatch, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::PostSettle {
            action: "tap".into(), duration_ms: 12, stable: true,
        }) {
            SubstepDetail::PostSettle { action, duration_ms, stable } => {
                assert_eq!(action, "tap");
                assert_eq!(duration_ms, 12);
                assert!(stable);
            }
            other => panic!("SHALL produce PostSettle, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::AppStop { bundle: "com.x".into() }) {
            SubstepDetail::AppStop { bundle } => assert_eq!(bundle, "com.x"),
            other => panic!("SHALL produce AppStop, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::DriverWarning { message: "slow".into() }) {
            SubstepDetail::DriverWarning { message } => assert_eq!(message, "slow"),
            other => panic!("SHALL produce DriverWarning, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::Screenshot { path: "/tmp/a.png".into() }) {
            SubstepDetail::Screenshot { path } => assert_eq!(path, "/tmp/a.png"),
            other => panic!("SHALL produce Screenshot, got {other:?}"),
        }
        match SubstepDetail::from(&SubstepEvent::Backspace { count: 3 }) {
            SubstepDetail::Backspace { count } => assert_eq!(count, 3),
            other => panic!("SHALL produce Backspace, got {other:?}"),
        }
    }

    // 25. Serialize emits the snake_case `type` tag and, via
    //     skip_serializing_if, omits a None element_bounds entirely while
    //     keeping a Some one.
    #[test]
    fn serialize_uses_snake_case_tag_and_skips_none_bounds() {
        let none = SubstepDetail::Tap {
            point: Point { x: 1, y: 2 },
            element_bounds: None,
        };
        let v = serde_json::to_value(&none).expect("SHALL serialize Tap");
        assert_eq!(v["type"], "tap", "SHALL tag with snake_case variant name");
        assert!(v.get("element_bounds").is_none(),
            "SHALL omit a None element_bounds entirely");

        let some = SubstepDetail::Tap {
            point: Point { x: 1, y: 2 },
            element_bounds: Some(Rect { x: 0, y: 0, width: 10, height: 10 }),
        };
        let v = serde_json::to_value(&some).expect("SHALL serialize Tap");
        assert!(v.get("element_bounds").is_some(),
            "SHALL keep a Some element_bounds");
    }

    // 26. The multi-word variant ElementNotFound serializes its tag in
    //     snake_case (element_not_found).
    #[test]
    fn serialize_multiword_variant_tag_is_snake_case() {
        let d = SubstepDetail::ElementNotFound {
            selector: "text=Ghost".into(),
            timeout_ms: 5000,
        };
        let v = serde_json::to_value(&d).expect("SHALL serialize ElementNotFound");
        assert_eq!(v["type"], "element_not_found",
            "SHALL render multi-word variant tag in snake_case");
        assert_eq!(v["timeout_ms"], 5000, "SHALL include scalar fields");
    }
}
