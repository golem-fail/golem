use std::collections::HashMap;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use golem_events::{DeviceId, Event, EventKind};
use tokio::sync::broadcast;

use crate::{FlowReport, InstallReport, StepOutcome, StepReport, SubstepDetail, SuiteReport};

/// Format a `SystemTime` as ISO-8601 UTC with millisecond precision:
/// `2026-04-22T14:32:15.123Z`.
fn iso8601_utc(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Resolve the failure code that surfaces on a flow's summary line.
///
/// An explicit code (set for flows that never ran a step, e.g.
/// `FlowCouldNotRun`) always wins; otherwise the code derived from the
/// first failed step is used. A `None` explicit code falls back to the
/// derived code, which may itself be `None` for an all-success flow.
fn resolve_flow_failure_code(
    explicit: Option<golem_events::FailureCode>,
    derived: Option<golem_events::FailureCode>,
) -> Option<golem_events::FailureCode> {
    explicit.or(derived)
}

struct AccumulatedStep {
    global_index: u64,
    block_name: String,
    /// Iteration of the containing block (0-based). Populated from the
    /// most recent `BlockStarted` on this device; used by TOON to render
    /// `B:block i:<N>` headers.
    block_iteration: u32,
    step_index_in_block: usize,
    action: String,
    selector_label: String,
    outcome: Option<golem_events::StepOutcome>,
    duration_ms: u64,
    retry_count: u32,
    screenshot_path: Option<String>,
    substeps: Vec<SubstepDetail>,
    tree_stats: golem_events::TreeStats,
    started_at: Option<SystemTime>,
    finished_at: Option<SystemTime>,
}

struct AccumulatedFlow {
    flow_name: String,
    device_id: DeviceId,
    os_major: Option<u32>,
    steps: Vec<AccumulatedStep>,
    warnings: Vec<String>,
    duration_ms: u64,
    success: bool,
    skipped_reason: Option<String>,
    started_at: Option<SystemTime>,
    finished_at: Option<SystemTime>,
    recordings: Vec<crate::RecordingEntry>,
    repeat: Option<golem_events::RepeatContext>,
    /// Set directly for flows that never ran a step (FlowCouldNotRun);
    /// otherwise derived from the first failed step in `into_suite_report`.
    first_failure_code: Option<golem_events::FailureCode>,
}

/// Accumulates events into a hierarchical SuiteReport.
#[derive(Default)]
pub struct ReportAccumulator {
    flows: Vec<AccumulatedFlow>,
    current_flow_by_device: HashMap<String, usize>, // device_id.0 -> index into flows
    current_step: HashMap<String, AccumulatedStep>, // current step per device
    /// Current block iteration per device, populated from `BlockStarted`
    /// events. Lets step-level renderers group consecutive steps under
    /// the correct block iteration even when blocks iterate.
    current_block_iter: HashMap<String, u32>,
    pub(crate) installs: Vec<InstallReport>,
    total_duration_ms: u64,
    /// Wall-clock of the first event observed. Used as suite started_at
    /// since no `SuiteStarted` event is emitted today.
    suite_started_at: Option<SystemTime>,
    /// Wall-clock of the `SuiteFinished` event.
    suite_finished_at: Option<SystemTime>,
    /// Per-install wall-clock tracking, keyed by (device_id, bundle_id).
    /// `InstallStarted` populates; `InstallFinished` drains into the final
    /// `InstallReport`.
    install_starts: HashMap<(String, String), SystemTime>,
}

impl ReportAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a single event.
    pub fn process(&mut self, event: &Event) {
        let dev_key = event.device_id.0.clone();

        // First event observed stamps the suite start — no `SuiteStarted`
        // event is emitted today; the plan-phase `SuitePlanned` or the
        // first device event is effectively "t=0".
        if self.suite_started_at.is_none() {
            self.suite_started_at = Some(event.wall_time);
        }

        match &event.kind {
            EventKind::FlowStarted {
                flow_name,
                os_major,
                repeat,
            } => {
                let idx = self.flows.len();
                self.flows.push(AccumulatedFlow {
                    flow_name: flow_name.clone(),
                    device_id: event.device_id.clone(),
                    os_major: Some(*os_major),
                    steps: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: 0,
                    success: true,
                    skipped_reason: None,
                    started_at: Some(event.wall_time),
                    finished_at: None,
                    recordings: Vec::new(),
                    repeat: *repeat,
                    first_failure_code: None,
                });
                self.current_flow_by_device.insert(dev_key, idx);
            }
            EventKind::FlowFinished {
                success,
                duration_ms,
                os_major,
                ..
            } => {
                if let Some(&idx) = self.current_flow_by_device.get(&dev_key) {
                    if let Some(flow) = self.flows.get_mut(idx) {
                        flow.success = *success;
                        flow.duration_ms = *duration_ms;
                        flow.finished_at = Some(event.wall_time);
                        if flow.os_major.is_none() {
                            flow.os_major = Some(*os_major);
                        }
                    }
                }
                self.current_flow_by_device.remove(&dev_key);
            }
            EventKind::FlowSkipped { flow_name, reason } => {
                // A deliberate skip or informational notice — never a failure
                // (coverage group satisfied, ANR-recovery reboot, etc.).
                // Recorded as a synthetic success=true flow so it shows in
                // reports without affecting the exit code.
                self.flows.push(AccumulatedFlow {
                    flow_name: flow_name.clone(),
                    device_id: event.device_id.clone(),
                    os_major: None,
                    steps: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: 0,
                    success: true,
                    skipped_reason: Some(reason.clone()),
                    started_at: Some(event.wall_time),
                    finished_at: Some(event.wall_time),
                    recordings: Vec::new(),
                    repeat: None,
                    first_failure_code: None,
                });
            }
            EventKind::FlowCouldNotRun {
                flow_name,
                reason,
                code,
            } => {
                // A precondition stopped the flow from running at all — it
                // executed no steps, so it counts as a failure (exit 1) and
                // carries the responsible code for the summary line.
                self.flows.push(AccumulatedFlow {
                    flow_name: flow_name.clone(),
                    device_id: event.device_id.clone(),
                    os_major: None,
                    steps: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: 0,
                    success: false,
                    skipped_reason: Some(reason.clone()),
                    first_failure_code: Some(*code),
                    started_at: Some(event.wall_time),
                    finished_at: Some(event.wall_time),
                    recordings: Vec::new(),
                    repeat: None,
                });
            }
            EventKind::BlockStarted { iteration, .. } => {
                self.current_block_iter.insert(dev_key, *iteration);
            }
            EventKind::BlockFinished {
                block_name,
                iteration,
                recording_path,
                ..
            } => {
                if let Some(path) = recording_path {
                    if let Some(&idx) = self.current_flow_by_device.get(&dev_key) {
                        if let Some(flow) = self.flows.get_mut(idx) {
                            flow.recordings.push(crate::RecordingEntry {
                                block: block_name.clone(),
                                iteration: *iteration,
                                path: path.clone(),
                            });
                        }
                    }
                }
            }
            EventKind::StepStarted {
                global_step_index,
                block_name,
                step_index_in_block,
                action,
                selector_label,
            } => {
                self.finish_current_step(&dev_key);
                let block_iteration = self.current_block_iter.get(&dev_key).copied().unwrap_or(0);
                self.current_step.insert(
                    dev_key,
                    AccumulatedStep {
                        global_index: *global_step_index,
                        block_name: block_name.clone(),
                        block_iteration,
                        step_index_in_block: *step_index_in_block,
                        action: action.clone(),
                        selector_label: selector_label.clone(),
                        outcome: None,
                        duration_ms: 0,
                        retry_count: 0,
                        screenshot_path: None,
                        substeps: Vec::new(),
                        tree_stats: golem_events::TreeStats::default(),
                        started_at: Some(event.wall_time),
                        finished_at: None,
                    },
                );
            }
            EventKind::StepFinished {
                outcome,
                duration_ms,
                retry_count,
                screenshot_path,
                tree_stats,
                ..
            } => {
                if let Some(step) = self.current_step.get_mut(&dev_key) {
                    step.outcome = Some(outcome.clone());
                    step.duration_ms = *duration_ms;
                    step.retry_count = *retry_count;
                    step.screenshot_path = screenshot_path.clone();
                    step.tree_stats = *tree_stats;
                    step.finished_at = Some(event.wall_time);

                    if let golem_events::StepOutcome::Warning { message, .. } = outcome {
                        if let Some(&idx) = self.current_flow_by_device.get(&dev_key) {
                            if let Some(flow) = self.flows.get_mut(idx) {
                                flow.warnings.push(message.clone());
                            }
                        }
                    }
                }
                self.finish_current_step(&dev_key);
            }
            EventKind::Substep(sub) => {
                if let Some(step) = self.current_step.get_mut(&dev_key) {
                    step.substeps.push(SubstepDetail::from(sub));
                }
            }
            EventKind::InstallStarted { bundle_id, .. } => {
                self.install_starts
                    .insert((dev_key.clone(), bundle_id.clone()), event.wall_time);
            }
            EventKind::InstallFinished {
                app_name,
                bundle_id,
                success,
                duration_ms,
                exit_code,
                error,
                code,
                target: _,
                os_major,
            } => {
                let started_at = self
                    .install_starts
                    .remove(&(dev_key.clone(), bundle_id.clone()))
                    .map(iso8601_utc);
                self.installs.push(InstallReport {
                    app_name: app_name.clone(),
                    bundle_id: bundle_id.clone(),
                    device_name: event.device_id.0.clone(),
                    os_major: Some(*os_major),
                    success: *success,
                    duration_ms: *duration_ms,
                    exit_code: *exit_code,
                    error: error.clone(),
                    code: *code,
                    started_at,
                    finished_at: Some(iso8601_utc(event.wall_time)),
                });
            }
            EventKind::SuiteFinished { duration_ms, .. } => {
                self.total_duration_ms = *duration_ms;
                self.suite_finished_at = Some(event.wall_time);
            }
            _ => {}
        }
    }

    fn finish_current_step(&mut self, dev_key: &str) {
        if let Some(step) = self.current_step.remove(dev_key) {
            if let Some(&idx) = self.current_flow_by_device.get(dev_key) {
                if let Some(flow) = self.flows.get_mut(idx) {
                    flow.steps.push(step);
                }
            }
        }
    }

    /// Convert accumulated data into a SuiteReport.
    pub fn into_suite_report(self) -> SuiteReport {
        let flows = self
            .flows
            .into_iter()
            .map(|flow| {
                // First *failed* step's code surfaces on the flow FAIL line. A
                // warning doesn't fail the flow, so its code must not pre-empt the
                // fatal step's code on the summary line. A flow that never ran
                // (FlowCouldNotRun) carries its code explicitly — that wins.
                let explicit_code = flow.first_failure_code;
                let mut first_failure_code: Option<golem_events::FailureCode> = None;
                let step_results = flow
                    .steps
                    .into_iter()
                    .map(|s| {
                        let outcome = match s.outcome {
                            Some(golem_events::StepOutcome::Success) => StepOutcome::Success,
                            Some(golem_events::StepOutcome::Warning { message, code }) => {
                                StepOutcome::Warning { message, code }
                            }
                            Some(golem_events::StepOutcome::Failed { message, code }) => {
                                first_failure_code.get_or_insert(code);
                                StepOutcome::Failed { message, code }
                            }
                            Some(golem_events::StepOutcome::Skipped) => StepOutcome::Skipped,
                            Some(golem_events::StepOutcome::Ignored) => StepOutcome::Skipped,
                            None => StepOutcome::Skipped,
                        };
                        StepReport {
                            global_step_index: s.global_index,
                            block_name: s.block_name,
                            block_iteration: s.block_iteration,
                            step_index_in_block: s.step_index_in_block,
                            action: s.action,
                            target: s.selector_label,
                            outcome,
                            duration_ms: s.duration_ms,
                            retry_count: s.retry_count,
                            screenshot_path: s.screenshot_path,
                            substeps: s.substeps,
                            tree_stats: s.tree_stats,
                            started_at: s.started_at.map(iso8601_utc),
                            finished_at: s.finished_at.map(iso8601_utc),
                        }
                    })
                    .collect();

                FlowReport {
                    flow_name: flow.flow_name,
                    success: flow.success,
                    skipped_reason: flow.skipped_reason,
                    step_results,
                    warnings: flow.warnings,
                    duration_ms: flow.duration_ms,
                    seed: None,
                    screenshot_path: None,
                    device_name: Some(flow.device_id.0),
                    os_major: flow.os_major,
                    perf_snapshots: Vec::new(),
                    covered_axes: Vec::new(),
                    recordings: flow.recordings,
                    started_at: flow.started_at.map(iso8601_utc),
                    finished_at: flow.finished_at.map(iso8601_utc),
                    repeat: flow.repeat,
                    first_failure_code: resolve_flow_failure_code(
                        explicit_code,
                        first_failure_code,
                    ),
                    // a11y audits travel the direct executor→FlowReport path
                    // (suite.rs), not the event stream — same as perf_snapshots.
                    a11y_audits: Vec::new(),
                }
            })
            .collect();

        SuiteReport {
            flows,
            installs: self.installs.clone(),
            total_duration_ms: self.total_duration_ms,
            started_at: self.suite_started_at.map(iso8601_utc),
            finished_at: self.suite_finished_at.map(iso8601_utc),
        }
    }
}

/// Run the accumulator as an async task consuming from a broadcast channel.
pub async fn accumulate_events(
    mut rx: broadcast::Receiver<Event>,
    accumulator: &tokio::sync::Mutex<ReportAccumulator>,
) {
    while let Ok(event) = rx.recv().await {
        accumulator.lock().await.process(&event);
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StepOutcome as ReportStepOutcome;
    use golem_events::{DeviceId, Event, EventKind, Point, Rect, SubstepEvent};
    use std::time::{Instant, SystemTime};

    fn make_event(seq: u64, device: &str, kind: EventKind) -> Event {
        Event {
            seq,
            device_id: DeviceId(device.into()),
            timestamp: Instant::now(),
            wall_time: SystemTime::now(),
            kind,
        }
    }

    // -- FlowStarted creates new flow entry --

    #[test]
    fn flow_started_creates_new_flow_entry() {
        let mut acc = ReportAccumulator::new();
        acc.process(&make_event(
            0,
            "dev1",
            EventKind::FlowStarted {
                flow_name: "login".into(),
                os_major: 0,
                repeat: None,
            },
        ));

        let report = acc.into_suite_report();
        assert_eq!(report.flows.len(), 1, "SHALL create one flow entry");
        assert_eq!(report.flows[0].flow_name, "login", "SHALL set flow name");
        assert_eq!(
            report.flows[0].device_name.as_deref(),
            Some("dev1"),
            "SHALL set device name from DeviceId"
        );
    }

    // -- FlowSkipped is a non-failing notice; FlowCouldNotRun is a failure --

    #[test]
    fn flow_skipped_is_success_not_a_failure() {
        let mut acc = ReportAccumulator::new();
        acc.process(&make_event(
            0,
            "dev1",
            EventKind::FlowSkipped {
                flow_name: "spared".into(),
                reason: "coverage group satisfied by peer run".into(),
            },
        ));
        let report = acc.into_suite_report();
        let flow = &report.flows[0];
        assert!(flow.is_skipped(), "FlowSkipped SHALL be a skip (success)");
        assert!(
            !flow.is_failed(),
            "FlowSkipped SHALL NOT count as a failure"
        );
        assert_eq!(flow.first_failure_code, None);
    }

    #[test]
    fn flow_could_not_run_is_a_failure_with_code() {
        let mut acc = ReportAccumulator::new();
        acc.process(&make_event(
            0,
            "dev1",
            EventKind::FlowCouldNotRun {
                flow_name: "needs_app".into(),
                reason: "install_script failed".into(),
                code: golem_events::FailureCode::AppInstallFailed,
            },
        ));
        let report = acc.into_suite_report();
        let flow = &report.flows[0];
        assert!(flow.is_failed(), "FlowCouldNotRun SHALL count as a failure");
        assert!(!flow.is_skipped(), "FlowCouldNotRun SHALL NOT be a skip");
        assert_eq!(
            flow.first_failure_code,
            Some(golem_events::FailureCode::AppInstallFailed),
            "FlowCouldNotRun SHALL carry its code onto the flow"
        );
    }

    // -- StepStarted + StepFinished produces correct StepReport --

    #[test]
    fn step_started_and_finished_produces_step_report() {
        let mut acc = ReportAccumulator::new();
        let dev = "pixel";

        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f1".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "main".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "Sign Up".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 45,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let report = acc.into_suite_report();
        let step = &report.flows[0].step_results[0];

        assert_eq!(
            step.global_step_index, 0,
            "SHALL preserve global_step_index"
        );
        assert_eq!(step.block_name, "main", "SHALL preserve block_name");
        assert_eq!(
            step.step_index_in_block, 0,
            "SHALL preserve step_index_in_block"
        );
        assert_eq!(step.action, "tap", "SHALL preserve action");
        assert_eq!(step.target, "Sign Up", "SHALL map selector_label to target");
        assert_eq!(step.duration_ms, 45, "SHALL preserve duration_ms");
        assert_eq!(step.retry_count, 0, "SHALL preserve retry_count");
        assert!(
            matches!(step.outcome, ReportStepOutcome::Success),
            "SHALL map Success outcome"
        );
    }

    // -- Substep events collected into current step's substeps vec --

    #[test]
    fn substep_events_collected_into_step_substeps() {
        let mut acc = ReportAccumulator::new();
        let dev = "iphone";

        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "OK".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::Substep(SubstepEvent::ElementResolved {
                selector: "text=OK".into(),
                bounds: Rect {
                    x: 10,
                    y: 20,
                    width: 100,
                    height: 40,
                },
                tap_point: Point { x: 60, y: 40 },
            }),
        ));
        acc.process(&make_event(
            3,
            dev,
            EventKind::Substep(SubstepEvent::Tap {
                point: Point { x: 60, y: 40 },
                element_bounds: Some(Rect {
                    x: 10,
                    y: 20,
                    width: 100,
                    height: 40,
                }),
            }),
        ));
        acc.process(&make_event(
            4,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 30,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let report = acc.into_suite_report();
        let step = &report.flows[0].step_results[0];

        assert_eq!(step.substeps.len(), 2, "SHALL collect both substep events");
        assert!(
            matches!(&step.substeps[0], SubstepDetail::ElementResolved { .. }),
            "SHALL convert first substep to ElementResolved"
        );
        assert!(
            matches!(&step.substeps[1], SubstepDetail::Tap { .. }),
            "SHALL convert second substep to Tap"
        );
    }

    // -- FlowFinished sets success/duration --

    #[test]
    fn flow_finished_sets_success_and_duration() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::FlowFinished {
                flow_name: "f".into(),
                success: false,
                duration_ms: 5000,
                seed: 0,
                os_major: 0,
                code: None,
                repeat: None,
            },
        ));

        let report = acc.into_suite_report();
        assert!(
            !report.flows[0].success,
            "SHALL set success=false from FlowFinished"
        );
        assert_eq!(
            report.flows[0].duration_ms, 5000,
            "SHALL set duration_ms from FlowFinished"
        );
    }

    // -- into_suite_report produces correct structure --

    #[test]
    fn into_suite_report_structure_with_steps_populated() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "checkout".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "main".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "Buy".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 20,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));
        acc.process(&make_event(
            3,
            dev,
            EventKind::StepStarted {
                global_step_index: 1,
                block_name: "main".into(),
                step_index_in_block: 1,
                action: "assert_visible".into(),
                selector_label: "Thank You".into(),
            },
        ));
        acc.process(&make_event(
            4,
            dev,
            EventKind::StepFinished {
                global_step_index: 1,
                outcome: golem_events::StepOutcome::Failed {
                    message: "not found".into(),
                    code: golem_events::FailureCode::FlowElementNotFound,
                },
                duration_ms: 10000,
                retry_count: 3,
                screenshot_path: Some("/tmp/fail.png".into()),
                tree_stats: golem_events::TreeStats::default(),
            },
        ));
        acc.process(&make_event(
            5,
            dev,
            EventKind::FlowFinished {
                flow_name: "checkout".into(),
                success: false,
                duration_ms: 10020,
                seed: 0,
                os_major: 0,
                code: None,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            6,
            dev,
            EventKind::SuiteFinished {
                duration_ms: 10020,
                passed: 0,
                failed: 1,
                skipped: 0,
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(
            suite.total_duration_ms, 10020,
            "SHALL set total_duration_ms from SuiteFinished"
        );
        assert_eq!(suite.flows.len(), 1, "SHALL have one flow");

        let flow = &suite.flows[0];
        assert_eq!(flow.step_results.len(), 2, "SHALL have two steps");
        assert_eq!(flow.step_results[0].action, "tap");
        assert_eq!(flow.step_results[1].action, "assert_visible");
        assert!(
            matches!(flow.step_results[1].outcome, ReportStepOutcome::Failed { ref message, .. } if message == "not found"),
            "SHALL preserve failure message"
        );
        assert_eq!(
            flow.step_results[1].retry_count, 3,
            "SHALL preserve retry_count"
        );
        assert_eq!(
            flow.step_results[1].screenshot_path.as_deref(),
            Some("/tmp/fail.png"),
            "SHALL preserve screenshot_path"
        );
    }

    // -- Multiple devices accumulate independently --

    #[test]
    fn multiple_devices_accumulate_independently() {
        let mut acc = ReportAccumulator::new();

        acc.process(&make_event(
            0,
            "ios",
            EventKind::FlowStarted {
                flow_name: "login_ios".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            "android",
            EventKind::FlowStarted {
                flow_name: "login_android".into(),
                os_major: 0,
                repeat: None,
            },
        ));

        // iOS step
        acc.process(&make_event(
            2,
            "ios",
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "OK".into(),
            },
        ));
        acc.process(&make_event(
            3,
            "ios",
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 30,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        // Android step
        acc.process(&make_event(
            4,
            "android",
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "type".into(),
                selector_label: "email".into(),
            },
        ));
        acc.process(&make_event(
            5,
            "android",
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Warning {
                    message: "slow".into(),
                    code: golem_events::FailureCode::Uncoded,
                },
                duration_ms: 200,
                retry_count: 1,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        acc.process(&make_event(
            6,
            "ios",
            EventKind::FlowFinished {
                flow_name: "login_ios".into(),
                success: true,
                duration_ms: 30,
                seed: 0,
                os_major: 0,
                code: None,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            7,
            "android",
            EventKind::FlowFinished {
                flow_name: "login_android".into(),
                success: true,
                duration_ms: 200,
                seed: 0,
                os_major: 0,
                code: None,
                repeat: None,
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(suite.flows.len(), 2, "SHALL have two separate flows");

        // Find each flow by device name
        let ios_flow = suite
            .flows
            .iter()
            .find(|f| f.device_name.as_deref() == Some("ios"))
            .expect("SHALL have iOS flow");
        let android_flow = suite
            .flows
            .iter()
            .find(|f| f.device_name.as_deref() == Some("android"))
            .expect("SHALL have Android flow");

        assert_eq!(ios_flow.step_results.len(), 1, "iOS SHALL have 1 step");
        assert_eq!(ios_flow.step_results[0].action, "tap");
        assert_eq!(
            android_flow.step_results.len(),
            1,
            "Android SHALL have 1 step"
        );
        assert_eq!(android_flow.step_results[0].action, "type");
    }

    // -- Warning outcome adds to flow warnings --

    #[test]
    fn warning_outcome_adds_to_flow_warnings() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "assert".into(),
                selector_label: "x".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Warning {
                    message: "flaky element".into(),
                    code: golem_events::FailureCode::Uncoded,
                },
                duration_ms: 50,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(suite.flows[0].warnings.len(), 1, "SHALL collect warning");
        assert_eq!(
            suite.flows[0].warnings[0], "flaky element",
            "SHALL preserve warning message"
        );
    }

    // -- Ignored and Skipped outcomes both map to Skipped --

    #[test]
    fn ignored_and_skipped_outcomes_map_to_skipped() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));

        // Skipped step
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "a".into(),
                selector_label: "s".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Skipped,
                duration_ms: 0,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        // Ignored step
        acc.process(&make_event(
            3,
            dev,
            EventKind::StepStarted {
                global_step_index: 1,
                block_name: "b".into(),
                step_index_in_block: 1,
                action: "b".into(),
                selector_label: "t".into(),
            },
        ));
        acc.process(&make_event(
            4,
            dev,
            EventKind::StepFinished {
                global_step_index: 1,
                outcome: golem_events::StepOutcome::Ignored,
                duration_ms: 0,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert!(
            matches!(
                suite.flows[0].step_results[0].outcome,
                ReportStepOutcome::Skipped
            ),
            "Skipped SHALL map to Skipped"
        );
        assert!(
            matches!(
                suite.flows[0].step_results[1].outcome,
                ReportStepOutcome::Skipped
            ),
            "Ignored SHALL map to Skipped"
        );
    }

    // 1. iso8601_utc renders millisecond-precision UTC with a trailing Z.
    #[test]
    fn iso8601_utc_has_millis_and_z_suffix() {
        // UNIX_EPOCH + 1500ms => 1970-01-01T00:00:01.500Z
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1500);
        let s = iso8601_utc(t);
        assert_eq!(
            s, "1970-01-01T00:00:01.500Z",
            "iso8601_utc SHALL render millisecond precision with a Z suffix"
        );
    }

    // 2. A step started but never finished before the flow ends maps to
    //    Skipped (outcome stays None and is converted on report build).
    #[test]
    fn unfinished_step_maps_to_skipped() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "X".into(),
            },
        ));
        // A new StepStarted finishes the previous (still-pending) step.
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepStarted {
                global_step_index: 1,
                block_name: "b".into(),
                step_index_in_block: 1,
                action: "tap".into(),
                selector_label: "Y".into(),
            },
        ));
        acc.process(&make_event(
            3,
            dev,
            EventKind::StepFinished {
                global_step_index: 1,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 5,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        let steps = &suite.flows[0].step_results;
        assert_eq!(steps.len(), 2, "both steps SHALL be recorded");
        assert!(
            matches!(steps[0].outcome, ReportStepOutcome::Skipped),
            "an unfinished step SHALL map to Skipped"
        );
        assert!(
            matches!(steps[1].outcome, ReportStepOutcome::Success),
            "the finished step SHALL keep its Success outcome"
        );
    }

    // 3. BlockStarted iteration is stamped onto subsequent steps' block_iteration.
    #[test]
    fn block_iteration_propagates_to_steps() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::BlockStarted {
                block_name: "loop".into(),
                block_index: 0,
                iteration: 3,
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "loop".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "X".into(),
            },
        ));
        acc.process(&make_event(
            3,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(
            suite.flows[0].step_results[0].block_iteration, 3,
            "step SHALL inherit the most recent BlockStarted iteration"
        );
    }

    // 4. With no BlockStarted, block_iteration defaults to 0.
    #[test]
    fn block_iteration_defaults_to_zero_without_block_started() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "X".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(
            suite.flows[0].step_results[0].block_iteration, 0,
            "absent BlockStarted SHALL default block_iteration to 0"
        );
    }

    // 5. BlockFinished with a recording_path appends a RecordingEntry to the flow.
    #[test]
    fn block_finished_with_recording_path_appends_recording() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::BlockFinished {
                block_name: "rec".into(),
                block_index: 0,
                iteration: 2,
                recording_path: Some("/tmp/rec.mp4".into()),
            },
        ));

        let suite = acc.into_suite_report();
        let recs = &suite.flows[0].recordings;
        assert_eq!(recs.len(), 1, "SHALL record one recording entry");
        assert_eq!(recs[0].block, "rec", "SHALL preserve block name");
        assert_eq!(recs[0].iteration, 2, "SHALL preserve iteration");
        assert_eq!(recs[0].path, "/tmp/rec.mp4", "SHALL preserve path");
    }

    // 6. BlockFinished with recording_path=None adds no recording.
    #[test]
    fn block_finished_without_recording_path_adds_nothing() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::BlockFinished {
                block_name: "rec".into(),
                block_index: 0,
                iteration: 0,
                recording_path: None,
            },
        ));

        let suite = acc.into_suite_report();
        assert!(
            suite.flows[0].recordings.is_empty(),
            "BlockFinished without a recording_path SHALL add no recording"
        );
    }

    // 7. InstallStarted then InstallFinished produces an InstallReport with
    //    both timestamps populated (started drained from install_starts).
    #[test]
    fn install_started_then_finished_populates_install_report() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::InstallStarted {
                app_name: "App".into(),
                bundle_id: "com.app".into(),
                script_path: "/s.sh".into(),
                target: "iPhone".into(),
                os_major: 18,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::InstallFinished {
                app_name: "App".into(),
                bundle_id: "com.app".into(),
                success: true,
                duration_ms: 1234,
                exit_code: None,
                error: None,
                code: None,
                target: "iPhone".into(),
                os_major: 18,
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(suite.installs.len(), 1, "SHALL produce one install report");
        let install = &suite.installs[0];
        assert_eq!(install.app_name, "App");
        assert_eq!(install.bundle_id, "com.app");
        assert_eq!(
            install.device_name, "dev",
            "SHALL set device_name from DeviceId"
        );
        assert_eq!(install.os_major, Some(18));
        assert!(install.success);
        assert_eq!(install.duration_ms, 1234);
        assert!(
            install.started_at.is_some(),
            "started_at SHALL be drained from InstallStarted"
        );
        assert!(
            install.finished_at.is_some(),
            "finished_at SHALL be stamped from InstallFinished"
        );
    }

    // 8. InstallFinished without a matching InstallStarted leaves started_at None.
    #[test]
    fn install_finished_without_start_has_no_started_at() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::InstallFinished {
                app_name: "App".into(),
                bundle_id: "com.app".into(),
                success: false,
                duration_ms: 0,
                exit_code: Some(2),
                error: Some("boom".into()),
                code: Some(golem_events::FailureCode::AppInstallFailed),
                target: "iPhone".into(),
                os_major: 18,
            },
        ));

        let suite = acc.into_suite_report();
        let install = &suite.installs[0];
        assert_eq!(
            install.started_at, None,
            "absent InstallStarted SHALL leave started_at None"
        );
        assert!(
            install.finished_at.is_some(),
            "finished_at SHALL still be stamped"
        );
        assert_eq!(install.exit_code, Some(2), "SHALL preserve exit_code");
        assert_eq!(
            install.error.as_deref(),
            Some("boom"),
            "SHALL preserve error"
        );
        assert_eq!(
            install.code,
            Some(golem_events::FailureCode::AppInstallFailed),
            "SHALL preserve code"
        );
    }

    // 9. The first *failed* step's code surfaces on the flow; a later failure
    //    does not override the earlier one.
    #[test]
    fn first_failed_step_code_wins_on_flow() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        // First failure
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "X".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Failed {
                    message: "first".into(),
                    code: golem_events::FailureCode::FlowElementNotFound,
                },
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));
        // Second failure with a different code
        acc.process(&make_event(
            3,
            dev,
            EventKind::StepStarted {
                global_step_index: 1,
                block_name: "b".into(),
                step_index_in_block: 1,
                action: "tap".into(),
                selector_label: "Y".into(),
            },
        ));
        acc.process(&make_event(
            4,
            dev,
            EventKind::StepFinished {
                global_step_index: 1,
                outcome: golem_events::StepOutcome::Failed {
                    message: "second".into(),
                    code: golem_events::FailureCode::Uncoded,
                },
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(
            suite.flows[0].first_failure_code,
            Some(golem_events::FailureCode::FlowElementNotFound),
            "the first failed step's code SHALL win"
        );
    }

    // 10. A warning's code SHALL NOT pre-empt a later failed step's code on
    //     the flow summary line.
    #[test]
    fn warning_code_does_not_preempt_failure_code() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        // Warning first
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "X".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Warning {
                    message: "warn".into(),
                    code: golem_events::FailureCode::Uncoded,
                },
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));
        // Real failure after
        acc.process(&make_event(
            3,
            dev,
            EventKind::StepStarted {
                global_step_index: 1,
                block_name: "b".into(),
                step_index_in_block: 1,
                action: "tap".into(),
                selector_label: "Y".into(),
            },
        ));
        acc.process(&make_event(
            4,
            dev,
            EventKind::StepFinished {
                global_step_index: 1,
                outcome: golem_events::StepOutcome::Failed {
                    message: "fail".into(),
                    code: golem_events::FailureCode::FlowElementNotFound,
                },
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(
            suite.flows[0].first_failure_code,
            Some(golem_events::FailureCode::FlowElementNotFound),
            "a warning code SHALL NOT pre-empt the fatal step's code"
        );
    }

    // 11. A flow whose steps all succeed derives no failure code: with no
    //     Failed step the derived code stays None and (absent an explicit
    //     FlowCouldNotRun code) `explicit_code.or(derived)` resolves to None.
    //     (The explicit-code-wins arm is covered by
    //     `flow_could_not_run_is_a_failure_with_code`, since a synthetic
    //     FlowCouldNotRun flow can carry no steps to derive a code from.)
    #[test]
    fn flow_with_only_success_steps_has_no_failure_code() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepStarted {
                global_step_index: 0,
                block_name: "b".into(),
                step_index_in_block: 0,
                action: "tap".into(),
                selector_label: "X".into(),
            },
        ));
        acc.process(&make_event(
            2,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(
            suite.flows[0].first_failure_code, None,
            "a flow with only successful steps SHALL carry no failure code"
        );
    }

    // 12. RepeatContext from FlowStarted is preserved onto the FlowReport.
    #[test]
    fn repeat_context_propagates_to_flow_report() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: Some(golem_events::RepeatContext { index: 2, total: 5 }),
            },
        ));

        let suite = acc.into_suite_report();
        let repeat = suite.flows[0].repeat.expect("repeat SHALL be preserved");
        assert_eq!(repeat.index, 2, "SHALL preserve repeat index");
        assert_eq!(repeat.total, 5, "SHALL preserve repeat total");
    }

    // 13. The first observed event stamps the suite started_at; SuiteFinished
    //     stamps finished_at. Both render as ISO-8601 UTC strings.
    #[test]
    fn suite_timestamps_from_first_event_and_suite_finished() {
        let mut acc = ReportAccumulator::new();
        let first = make_event(
            0,
            "dev",
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        );
        let started_iso = iso8601_utc(first.wall_time);
        acc.process(&first);
        acc.process(&make_event(
            1,
            "dev",
            EventKind::SuiteFinished {
                duration_ms: 99,
                passed: 0,
                failed: 0,
                skipped: 0,
            },
        ));

        let suite = acc.into_suite_report();
        assert_eq!(suite.total_duration_ms, 99, "SHALL set total_duration_ms");
        assert_eq!(
            suite.started_at.as_deref(),
            Some(started_iso.as_str()),
            "started_at SHALL come from the first observed event's wall_time"
        );
        assert!(
            suite.finished_at.is_some(),
            "finished_at SHALL be set from SuiteFinished"
        );
    }

    // 14. Substep events arriving with no current step are dropped silently
    //     (no current step => nowhere to attach).
    #[test]
    fn substep_without_current_step_is_dropped() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        // No StepStarted; this substep has nowhere to land.
        acc.process(&make_event(
            1,
            dev,
            EventKind::Substep(SubstepEvent::Tap {
                point: Point { x: 1, y: 2 },
                element_bounds: None,
            }),
        ));

        let suite = acc.into_suite_report();
        assert!(
            suite.flows[0].step_results.is_empty(),
            "a substep with no current step SHALL be dropped, creating no step"
        );
    }

    // 15. StepFinished arriving with no current step is a no-op (no flow steps).
    #[test]
    fn step_finished_without_current_step_is_noop() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";
        acc.process(&make_event(
            0,
            dev,
            EventKind::FlowStarted {
                flow_name: "f".into(),
                os_major: 0,
                repeat: None,
            },
        ));
        acc.process(&make_event(
            1,
            dev,
            EventKind::StepFinished {
                global_step_index: 0,
                outcome: golem_events::StepOutcome::Success,
                duration_ms: 1,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: golem_events::TreeStats::default(),
            },
        ));

        let suite = acc.into_suite_report();
        assert!(
            suite.flows[0].step_results.is_empty(),
            "StepFinished with no current step SHALL add no step"
        );
    }

    // 17. An explicit code (FlowCouldNotRun) SHALL win over a co-present
    //     derived step code — the precedence the side-effecting accumulator
    //     cannot exercise directly (a synthetic FlowCouldNotRun flow carries
    //     no steps). The extracted pure helper lets us assert it head-on.
    #[test]
    fn explicit_failure_code_wins_over_derived() {
        let explicit = Some(golem_events::FailureCode::AppInstallFailed);
        let derived = Some(golem_events::FailureCode::FlowElementNotFound);
        assert_eq!(
            resolve_flow_failure_code(explicit, derived),
            Some(golem_events::FailureCode::AppInstallFailed),
            "an explicit code SHALL win over a co-present derived code"
        );
    }

    // 18. Absent an explicit code, the derived step code SHALL surface.
    #[test]
    fn derived_failure_code_used_when_no_explicit() {
        let derived = Some(golem_events::FailureCode::FlowElementNotFound);
        assert_eq!(
            resolve_flow_failure_code(None, derived),
            Some(golem_events::FailureCode::FlowElementNotFound),
            "the derived code SHALL surface when no explicit code is set"
        );
    }

    // 19. With neither an explicit nor a derived code the result SHALL be None
    //     (an all-success flow carries no failure code).
    #[test]
    fn no_failure_code_resolves_to_none() {
        assert_eq!(
            resolve_flow_failure_code(None, None),
            None,
            "absent both codes the flow SHALL carry no failure code"
        );
    }

    // 16. accumulate_events drains the broadcast channel into the accumulator
    //     and stops when the sender is dropped.
    #[tokio::test]
    async fn accumulate_events_drains_channel_until_closed() {
        let (tx, rx) = broadcast::channel(16);
        let acc = tokio::sync::Mutex::new(ReportAccumulator::new());

        tx.send(make_event(
            0,
            "dev",
            EventKind::FlowStarted {
                flow_name: "drained".into(),
                os_major: 0,
                repeat: None,
            },
        ))
        .expect("send SHALL succeed");
        drop(tx); // closing the channel ends the loop

        accumulate_events(rx, &acc).await;

        let report = acc.into_inner().into_suite_report();
        assert_eq!(report.flows.len(), 1, "SHALL accumulate the sent event");
        assert_eq!(
            report.flows[0].flow_name, "drained",
            "SHALL process the event payload"
        );
    }
}
