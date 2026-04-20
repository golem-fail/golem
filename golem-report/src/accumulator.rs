use std::collections::HashMap;

use golem_events::{DeviceId, Event, EventKind};
use tokio::sync::broadcast;

use crate::{FlowReport, StepOutcome, StepReport, SubstepDetail, SuiteReport};

struct AccumulatedStep {
    global_index: u64,
    block_name: String,
    step_index_in_block: usize,
    action: String,
    selector_label: String,
    outcome: Option<golem_events::StepOutcome>,
    duration_ms: u64,
    retry_count: u32,
    screenshot_path: Option<String>,
    substeps: Vec<SubstepDetail>,
}

struct AccumulatedFlow {
    flow_name: String,
    device_id: DeviceId,
    steps: Vec<AccumulatedStep>,
    warnings: Vec<String>,
    duration_ms: u64,
    success: bool,
}

/// Accumulates events into a hierarchical SuiteReport.
pub struct ReportAccumulator {
    flows: HashMap<String, AccumulatedFlow>, // keyed by device_id.0
    current_step: HashMap<String, AccumulatedStep>, // current step per device
    total_duration_ms: u64,
}

impl ReportAccumulator {
    pub fn new() -> Self {
        Self {
            flows: HashMap::new(),
            current_step: HashMap::new(),
            total_duration_ms: 0,
        }
    }

    /// Process a single event.
    pub fn process(&mut self, event: &Event) {
        let dev_key = event.device_id.0.clone();

        match &event.kind {
            EventKind::FlowStarted { flow_name } => {
                self.flows.insert(dev_key, AccumulatedFlow {
                    flow_name: flow_name.clone(),
                    device_id: event.device_id.clone(),
                    steps: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: 0,
                    success: true,
                });
            }
            EventKind::FlowFinished { success, duration_ms, .. } => {
                if let Some(flow) = self.flows.get_mut(&dev_key) {
                    flow.success = *success;
                    flow.duration_ms = *duration_ms;
                }
            }
            EventKind::StepStarted { global_step_index, block_name, step_index_in_block, action, selector_label } => {
                // Finish any previous step (shouldn't happen, but defensive)
                self.finish_current_step(&dev_key);
                self.current_step.insert(dev_key, AccumulatedStep {
                    global_index: *global_step_index,
                    block_name: block_name.clone(),
                    step_index_in_block: *step_index_in_block,
                    action: action.clone(),
                    selector_label: selector_label.clone(),
                    outcome: None,
                    duration_ms: 0,
                    retry_count: 0,
                    screenshot_path: None,
                    substeps: Vec::new(),
                });
            }
            EventKind::StepFinished { outcome, duration_ms, retry_count, screenshot_path, .. } => {
                if let Some(step) = self.current_step.get_mut(&dev_key) {
                    step.outcome = Some(outcome.clone());
                    step.duration_ms = *duration_ms;
                    step.retry_count = *retry_count;
                    step.screenshot_path = screenshot_path.clone();

                    // Collect warnings
                    if let golem_events::StepOutcome::Warning(msg) = outcome {
                        if let Some(flow) = self.flows.get_mut(&dev_key) {
                            flow.warnings.push(msg.clone());
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
            EventKind::SuiteFinished { duration_ms, .. } => {
                self.total_duration_ms = *duration_ms;
            }
            _ => {}
        }
    }

    fn finish_current_step(&mut self, dev_key: &str) {
        if let Some(step) = self.current_step.remove(dev_key) {
            if let Some(flow) = self.flows.get_mut(dev_key) {
                flow.steps.push(step);
            }
        }
    }

    /// Convert accumulated data into a SuiteReport.
    pub fn into_suite_report(self) -> SuiteReport {
        let flows = self.flows.into_values().map(|flow| {
            let step_results = flow.steps.into_iter().map(|s| {
                let outcome = match s.outcome {
                    Some(golem_events::StepOutcome::Success) => StepOutcome::Success,
                    Some(golem_events::StepOutcome::Warning(msg)) => StepOutcome::Warning(msg),
                    Some(golem_events::StepOutcome::Failed(msg)) => StepOutcome::Failed(msg),
                    Some(golem_events::StepOutcome::Skipped) => StepOutcome::Skipped,
                    Some(golem_events::StepOutcome::Ignored) => StepOutcome::Skipped,
                    None => StepOutcome::Skipped,
                };
                StepReport {
                    global_step_index: s.global_index,
                    block_name: s.block_name,
                    step_index_in_block: s.step_index_in_block,
                    action: s.action,
                    target: s.selector_label,
                    outcome,
                    duration_ms: s.duration_ms,
                    retry_count: s.retry_count,
                    screenshot_path: s.screenshot_path,
                    substeps: s.substeps,
                }
            }).collect();

            FlowReport {
                flow_name: flow.flow_name,
                success: flow.success,
                step_results,
                warnings: flow.warnings,
                duration_ms: flow.duration_ms,
                seed: None,
                screenshot_path: None,
                device_name: Some(flow.device_id.0),
                perf_snapshots: Vec::new(),
            }
        }).collect();

        SuiteReport {
            flows,
            total_duration_ms: self.total_duration_ms,
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
    use std::time::Instant;
    use golem_events::{DeviceId, Event, EventKind, Point, Rect, SubstepEvent};
    use crate::StepOutcome as ReportStepOutcome;

    fn make_event(seq: u64, device: &str, kind: EventKind) -> Event {
        Event {
            seq,
            device_id: DeviceId(device.into()),
            timestamp: Instant::now(),
            kind,
        }
    }

    // -- FlowStarted creates new flow entry --

    #[test]
    fn flow_started_creates_new_flow_entry() {
        let mut acc = ReportAccumulator::new();
        acc.process(&make_event(0, "dev1", EventKind::FlowStarted {
            flow_name: "login".into(),
        }));

        let report = acc.into_suite_report();
        assert_eq!(report.flows.len(), 1, "SHALL create one flow entry");
        assert_eq!(report.flows[0].flow_name, "login", "SHALL set flow name");
        assert_eq!(
            report.flows[0].device_name.as_deref(),
            Some("dev1"),
            "SHALL set device name from DeviceId"
        );
    }

    // -- StepStarted + StepFinished produces correct StepReport --

    #[test]
    fn step_started_and_finished_produces_step_report() {
        let mut acc = ReportAccumulator::new();
        let dev = "pixel";

        acc.process(&make_event(0, dev, EventKind::FlowStarted {
            flow_name: "f1".into(),
        }));
        acc.process(&make_event(1, dev, EventKind::StepStarted {
            global_step_index: 0,
            block_name: "main".into(),
            step_index_in_block: 0,
            action: "tap".into(),
            selector_label: "Sign Up".into(),
        }));
        acc.process(&make_event(2, dev, EventKind::StepFinished {
            global_step_index: 0,
            outcome: golem_events::StepOutcome::Success,
            duration_ms: 45,
            retry_count: 0,
            screenshot_path: None,
        }));

        let report = acc.into_suite_report();
        let step = &report.flows[0].step_results[0];

        assert_eq!(step.global_step_index, 0, "SHALL preserve global_step_index");
        assert_eq!(step.block_name, "main", "SHALL preserve block_name");
        assert_eq!(step.step_index_in_block, 0, "SHALL preserve step_index_in_block");
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

        acc.process(&make_event(0, dev, EventKind::FlowStarted {
            flow_name: "f".into(),
        }));
        acc.process(&make_event(1, dev, EventKind::StepStarted {
            global_step_index: 0,
            block_name: "b".into(),
            step_index_in_block: 0,
            action: "tap".into(),
            selector_label: "OK".into(),
        }));
        acc.process(&make_event(2, dev, EventKind::Substep(SubstepEvent::ElementResolved {
            selector: "text=OK".into(),
            bounds: Rect { x: 10, y: 20, width: 100, height: 40 },
            tap_point: Point { x: 60, y: 40 },
        })));
        acc.process(&make_event(3, dev, EventKind::Substep(SubstepEvent::Tap {
            point: Point { x: 60, y: 40 },
            element_bounds: Some(Rect { x: 10, y: 20, width: 100, height: 40 }),
        })));
        acc.process(&make_event(4, dev, EventKind::StepFinished {
            global_step_index: 0,
            outcome: golem_events::StepOutcome::Success,
            duration_ms: 30,
            retry_count: 0,
            screenshot_path: None,
        }));

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

        acc.process(&make_event(0, dev, EventKind::FlowStarted {
            flow_name: "f".into(),
        }));
        acc.process(&make_event(1, dev, EventKind::FlowFinished {
            flow_name: "f".into(),
            success: false,
            duration_ms: 5000,
        }));

        let report = acc.into_suite_report();
        assert!(!report.flows[0].success, "SHALL set success=false from FlowFinished");
        assert_eq!(report.flows[0].duration_ms, 5000, "SHALL set duration_ms from FlowFinished");
    }

    // -- into_suite_report produces correct structure --

    #[test]
    fn into_suite_report_structure_with_steps_populated() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(0, dev, EventKind::FlowStarted {
            flow_name: "checkout".into(),
        }));
        acc.process(&make_event(1, dev, EventKind::StepStarted {
            global_step_index: 0,
            block_name: "main".into(),
            step_index_in_block: 0,
            action: "tap".into(),
            selector_label: "Buy".into(),
        }));
        acc.process(&make_event(2, dev, EventKind::StepFinished {
            global_step_index: 0,
            outcome: golem_events::StepOutcome::Success,
            duration_ms: 20,
            retry_count: 0,
            screenshot_path: None,
        }));
        acc.process(&make_event(3, dev, EventKind::StepStarted {
            global_step_index: 1,
            block_name: "main".into(),
            step_index_in_block: 1,
            action: "assert_visible".into(),
            selector_label: "Thank You".into(),
        }));
        acc.process(&make_event(4, dev, EventKind::StepFinished {
            global_step_index: 1,
            outcome: golem_events::StepOutcome::Failed("not found".into()),
            duration_ms: 10000,
            retry_count: 3,
            screenshot_path: Some("/tmp/fail.png".into()),
        }));
        acc.process(&make_event(5, dev, EventKind::FlowFinished {
            flow_name: "checkout".into(),
            success: false,
            duration_ms: 10020,
        }));
        acc.process(&make_event(6, dev, EventKind::SuiteFinished {
            duration_ms: 10020,
            passed: 0,
            failed: 1,
        }));

        let suite = acc.into_suite_report();
        assert_eq!(suite.total_duration_ms, 10020, "SHALL set total_duration_ms from SuiteFinished");
        assert_eq!(suite.flows.len(), 1, "SHALL have one flow");

        let flow = &suite.flows[0];
        assert_eq!(flow.step_results.len(), 2, "SHALL have two steps");
        assert_eq!(flow.step_results[0].action, "tap");
        assert_eq!(flow.step_results[1].action, "assert_visible");
        assert!(
            matches!(flow.step_results[1].outcome, ReportStepOutcome::Failed(ref m) if m == "not found"),
            "SHALL preserve failure message"
        );
        assert_eq!(flow.step_results[1].retry_count, 3, "SHALL preserve retry_count");
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

        acc.process(&make_event(0, "ios", EventKind::FlowStarted {
            flow_name: "login_ios".into(),
        }));
        acc.process(&make_event(1, "android", EventKind::FlowStarted {
            flow_name: "login_android".into(),
        }));

        // iOS step
        acc.process(&make_event(2, "ios", EventKind::StepStarted {
            global_step_index: 0,
            block_name: "b".into(),
            step_index_in_block: 0,
            action: "tap".into(),
            selector_label: "OK".into(),
        }));
        acc.process(&make_event(3, "ios", EventKind::StepFinished {
            global_step_index: 0,
            outcome: golem_events::StepOutcome::Success,
            duration_ms: 30,
            retry_count: 0,
            screenshot_path: None,
        }));

        // Android step
        acc.process(&make_event(4, "android", EventKind::StepStarted {
            global_step_index: 0,
            block_name: "b".into(),
            step_index_in_block: 0,
            action: "type".into(),
            selector_label: "email".into(),
        }));
        acc.process(&make_event(5, "android", EventKind::StepFinished {
            global_step_index: 0,
            outcome: golem_events::StepOutcome::Warning("slow".into()),
            duration_ms: 200,
            retry_count: 1,
            screenshot_path: None,
        }));

        acc.process(&make_event(6, "ios", EventKind::FlowFinished {
            flow_name: "login_ios".into(), success: true, duration_ms: 30,
        }));
        acc.process(&make_event(7, "android", EventKind::FlowFinished {
            flow_name: "login_android".into(), success: true, duration_ms: 200,
        }));

        let suite = acc.into_suite_report();
        assert_eq!(suite.flows.len(), 2, "SHALL have two separate flows");

        // Find each flow by device name
        let ios_flow = suite.flows.iter().find(|f| f.device_name.as_deref() == Some("ios"))
            .expect("SHALL have iOS flow");
        let android_flow = suite.flows.iter().find(|f| f.device_name.as_deref() == Some("android"))
            .expect("SHALL have Android flow");

        assert_eq!(ios_flow.step_results.len(), 1, "iOS SHALL have 1 step");
        assert_eq!(ios_flow.step_results[0].action, "tap");
        assert_eq!(android_flow.step_results.len(), 1, "Android SHALL have 1 step");
        assert_eq!(android_flow.step_results[0].action, "type");
    }

    // -- Warning outcome adds to flow warnings --

    #[test]
    fn warning_outcome_adds_to_flow_warnings() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(0, dev, EventKind::FlowStarted {
            flow_name: "f".into(),
        }));
        acc.process(&make_event(1, dev, EventKind::StepStarted {
            global_step_index: 0,
            block_name: "b".into(),
            step_index_in_block: 0,
            action: "assert".into(),
            selector_label: "x".into(),
        }));
        acc.process(&make_event(2, dev, EventKind::StepFinished {
            global_step_index: 0,
            outcome: golem_events::StepOutcome::Warning("flaky element".into()),
            duration_ms: 50,
            retry_count: 0,
            screenshot_path: None,
        }));

        let suite = acc.into_suite_report();
        assert_eq!(suite.flows[0].warnings.len(), 1, "SHALL collect warning");
        assert_eq!(suite.flows[0].warnings[0], "flaky element", "SHALL preserve warning message");
    }

    // -- Ignored and Skipped outcomes both map to Skipped --

    #[test]
    fn ignored_and_skipped_outcomes_map_to_skipped() {
        let mut acc = ReportAccumulator::new();
        let dev = "dev";

        acc.process(&make_event(0, dev, EventKind::FlowStarted { flow_name: "f".into() }));

        // Skipped step
        acc.process(&make_event(1, dev, EventKind::StepStarted {
            global_step_index: 0, block_name: "b".into(), step_index_in_block: 0,
            action: "a".into(), selector_label: "s".into(),
        }));
        acc.process(&make_event(2, dev, EventKind::StepFinished {
            global_step_index: 0, outcome: golem_events::StepOutcome::Skipped,
            duration_ms: 0, retry_count: 0, screenshot_path: None,
        }));

        // Ignored step
        acc.process(&make_event(3, dev, EventKind::StepStarted {
            global_step_index: 1, block_name: "b".into(), step_index_in_block: 1,
            action: "b".into(), selector_label: "t".into(),
        }));
        acc.process(&make_event(4, dev, EventKind::StepFinished {
            global_step_index: 1, outcome: golem_events::StepOutcome::Ignored,
            duration_ms: 0, retry_count: 0, screenshot_path: None,
        }));

        let suite = acc.into_suite_report();
        assert!(
            matches!(suite.flows[0].step_results[0].outcome, ReportStepOutcome::Skipped),
            "Skipped SHALL map to Skipped"
        );
        assert!(
            matches!(suite.flows[0].step_results[1].outcome, ReportStepOutcome::Skipped),
            "Ignored SHALL map to Skipped"
        );
    }
}
