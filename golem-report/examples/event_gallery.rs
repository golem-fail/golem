//! Visual gallery of every `EventKind` (and every nested `SubstepEvent`) as
//! rendered by the live human streamer, for golem devs to eyeball colour and
//! layout. NOT part of the product — examples are only built by
//! `cargo run --example`, never linked into the release binary.
//!
//! Run it:
//!
//! ```text
//! cargo run -p golem-report --example event_gallery
//! ```
//!
//! Colour turns on automatically when stderr is a terminal (the renderer's own
//! `is_terminal` check), so run it directly in your shell to see the palette;
//! pipe it to a file for the plain-text form.
//!
//! In a real run many of these events are mutually exclusive per step (a step
//! either succeeds or fails; a scroll either finds or reverses). The gallery
//! deliberately emits all of them in one coherent-looking sequence so the eye
//! can compare every rendering side by side.
//!
//! ## Why the `_exhaustiveness_guard_*` fns exist
//!
//! `stream::stream_human` ends its `match` with `_ => {}`, so a newly added
//! `EventKind`/`SubstepEvent` that nobody wrote a render arm for prints
//! *nothing* — silently. The two guard fns below match every variant with no
//! wildcard, so adding a variant fails to compile here until a dev acknowledges
//! it. When that happens, also add a sample to `gallery()` so the new variant
//! stays visible in this gallery.

use golem_events::channel::event_channel;
use golem_events::{
    A11yAudit, A11yIssue, DeviceId, EventKind, FailureCode, PerfSnapshotData, Point, Rect,
    RepeatContext, ScrollAttemptResult, Severity, StepOutcome, SubstepEvent, TreeStats,
};
use golem_report::stream::stream_human;

const IOS: &str = "ios/iPhone 17";

fn dev() -> DeviceId {
    DeviceId(IOS.into())
}

fn suite() -> DeviceId {
    DeviceId("suite".into())
}

fn rect() -> Rect {
    Rect {
        x: 40,
        y: 120,
        width: 200,
        height: 48,
    }
}

fn point() -> Point {
    Point { x: 140, y: 144 }
}

/// Every `SubstepEvent`, in roughly the order a rich step would produce them.
/// Kept separate so the exhaustiveness guard and this list read against each
/// other easily.
fn all_substeps() -> Vec<SubstepEvent> {
    vec![
        SubstepEvent::ElementResolved {
            selector: "text=Submit".into(),
            bounds: rect(),
            tap_point: point(),
        },
        SubstepEvent::ElementNotFound {
            selector: "text=Ghost".into(),
            timeout_ms: 5_000,
        },
        SubstepEvent::Tap {
            point: point(),
            element_bounds: Some(rect()),
        },
        SubstepEvent::DoubleTap {
            point: point(),
            element_bounds: Some(rect()),
        },
        SubstepEvent::LongPress {
            point: point(),
            duration_ms: 800,
            element_bounds: Some(rect()),
        },
        SubstepEvent::TextInput {
            text: "hello@example.com".into(),
            field_bounds: Some(rect()),
        },
        SubstepEvent::Backspace { count: 3 },
        SubstepEvent::Swipe {
            from: Point { x: 200, y: 600 },
            to: Point { x: 200, y: 200 },
            duration_ms: Some(300),
        },
        SubstepEvent::ScrollStarted {
            selector: "text=Footer".into(),
            direction: "down".into(),
        },
        SubstepEvent::ScrollAttempt {
            attempt: 1,
            direction: "down".into(),
            strategy_index: 0,
            container: false,
            from: Point { x: 200, y: 600 },
            to: Point { x: 200, y: 200 },
            result: ScrollAttemptResult::PageScrolled,
            tree_stats: TreeStats {
                fetches: 2,
                min_nodes: 40,
                max_nodes: 46,
            },
        },
        SubstepEvent::ScrollFound {
            selector: "text=Footer".into(),
            position: Point { x: 200, y: 300 },
            total_attempts: 3,
        },
        SubstepEvent::ScrollDirectionReversed {
            to_direction: "up".into(),
            reason: "overshot target".into(),
        },
        SubstepEvent::ScrollStrategySwitch {
            to_index: 1,
            reason: "page scroll stalled".into(),
        },
        SubstepEvent::AssertionMatch {
            expected: "Counter".into(),
            actual: "Counter".into(),
            element_bounds: Some(rect()),
        },
        SubstepEvent::AssertionMismatch {
            expected: "Counter: 2".into(),
            actual: Some("Counter: 1".into()),
        },
        SubstepEvent::AlertFound {
            text: Some("Allow notifications?".into()),
        },
        SubstepEvent::RetryAttempt {
            attempt: 2,
            max: 3,
            delay_ms: 500,
            error: "element not yet settled".into(),
        },
        SubstepEvent::HttpRequest {
            method: "GET".into(),
            url: "http://127.0.0.1:8251/hierarchy".into(),
            status: Some(200),
            duration_ms: 42,
        },
        SubstepEvent::BashCommand {
            command: "xcrun simctl list".into(),
            exit_code: Some(0),
            duration_ms: 120,
        },
        SubstepEvent::PostSettle {
            action: "tap".into(),
            duration_ms: 260,
            stable: true,
        },
        SubstepEvent::AppLaunch {
            bundle: "fail.golem.test".into(),
            duration_ms: 1_200,
        },
        SubstepEvent::AppStop {
            bundle: "fail.golem.test".into(),
        },
        SubstepEvent::DriverWarning {
            message: "CDP enrichment unavailable — falling back to native tree".into(),
        },
        SubstepEvent::Screenshot {
            path: ".golem/results/tap/ios/screenshots/step-3.png".into(),
        },
        SubstepEvent::BarrierAborted { step_count: 7 },
    ]
}

/// One representative `StepFinished` per `StepOutcome`, so each outcome's
/// colour/label is visible. `global_step_index` is offset past the main step.
fn outcome_steps() -> Vec<(DeviceId, EventKind)> {
    let outcomes = [
        ("assert_visible", "text=Counter", StepOutcome::Success),
        (
            "assert_visible",
            "text=Badge",
            StepOutcome::Warning {
                message: "matched a low-confidence element".into(),
                code: FailureCode::FlowAssertionMismatch,
            },
        ),
        (
            "assert_visible",
            "text=Missing",
            StepOutcome::Failed {
                message: "element never appeared".into(),
                code: FailureCode::FlowElementNotFound,
            },
        ),
        ("tap", "text=Skipped", StepOutcome::Skipped),
        ("tap", "text=Ignored", StepOutcome::Ignored),
    ];
    let mut out = Vec::new();
    for (i, (action, selector, outcome)) in outcomes.into_iter().enumerate() {
        let idx = 100 + i as u64;
        out.push((
            dev(),
            EventKind::StepStarted {
                global_step_index: idx,
                block_name: "outcomes".into(),
                step_index_in_block: i,
                action: action.into(),
                selector_label: selector.into(),
            },
        ));
        out.push((
            dev(),
            EventKind::StepFinished {
                global_step_index: idx,
                outcome,
                duration_ms: 120,
                retry_count: 0,
                screenshot_path: None,
                tree_stats: TreeStats::default(),
            },
        ));
    }
    out
}

/// The full ordered event sequence. Grouped like a real run: suite → setup →
/// install → flow/block/step (+ every substep) → outcomes → recovery → finish.
// Sequential pushes interleave with loops and `extend`, so a single `vec![]`
// literal (what the lint wants) can't express this.
#[allow(clippy::vec_init_then_push)]
fn gallery() -> Vec<(DeviceId, EventKind)> {
    let mut ev: Vec<(DeviceId, EventKind)> = Vec::new();

    // ── Suite ──
    ev.push((suite(), EventKind::SuiteStarted { flow_count: 2 }));
    ev.push((
        suite(),
        EventKind::SuiteLint {
            warnings: vec!["tap.test:12 `within` set on an action that ignores it".into()],
        },
    ));
    ev.push((
        suite(),
        EventKind::SuitePlanned {
            flow_runs: vec!["#1 tap.test: ios/v26 apps=[app]".into()],
            install_entries: vec!["ios app → fail.golem.test".into()],
            device_availability: vec!["ios/v26/phone — 1 device (1 booted)".into()],
        },
    ));

    // ── Setup narrative ──
    ev.push((
        suite(),
        EventKind::FlowParseFailed {
            path: "flows/broken.test.toml".into(),
            error: "expected a table key at line 4".into(),
        },
    ));
    ev.push((
        suite(),
        EventKind::DeviceAutoBoot {
            device_name: "iPhone 17".into(),
            slot_shape: "ios/v26/phone".into(),
        },
    ));
    ev.push((
        suite(),
        EventKind::DeviceAutoBootFinished {
            device_name: "iPhone 17".into(),
            slot_shape: "ios/v26/phone".into(),
            duration_ms: 8_400,
        },
    ));
    ev.push((
        suite(),
        EventKind::DeviceBootRequested {
            platform: "ios".into(),
        },
    ));
    ev.push((
        suite(),
        EventKind::SlotSetupFailed {
            slot_label: "ios/v26/phone apps=[app]".into(),
            reason: "no booted device and auto-create disabled".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::ResourcesWaiting {
            platform: "ios".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::CompanionStarting {
            platform: "ios".into(),
            device_name: "iPhone 17".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::CompanionRestarting {
            device_name: "iPhone 17".into(),
            attempt: 1,
            max: 2,
        },
    ));
    ev.push((
        dev(),
        EventKind::CompanionReady {
            platform: "ios".into(),
            version: "0.8.1".into(),
            device_name: "iPhone 17".into(),
            os_version: "26.5".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::RegistrationCompleted {
            device_name: "iPhone 17".into(),
            platform: "ios".into(),
            port: 8251,
        },
    ));
    ev.push((
        suite(),
        EventKind::RegistrationError {
            error: "invalid JSON in /register body".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::DeviceSettingsWarning {
            device_name: "iPhone 17".into(),
            warning: "could not disable keyboard autocorrect".into(),
        },
    ));
    ev.push((
        suite(),
        EventKind::InstallCacheFileBroken {
            path: ".golem/install-cache.json".into(),
            reason: "has unknown version 99 — treating as empty".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::InstallCacheWriteFailed {
            reason: "permission denied".into(),
        },
    ));

    // ── Install ──
    let target = "iPhone 17 (ios/v26/phone)";
    ev.push((
        dev(),
        EventKind::InstallCacheMiss {
            app_name: "app".into(),
            bundle_id: "fail.golem.test".into(),
            target: target.into(),
            reason: "source fingerprint changed (git:04eeec6 → git:1e11d64)".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::InstallSkipped {
            app_name: "app-b".into(),
            bundle_id: "fail.golem.test.b".into(),
            target: target.into(),
            reason: "cache hit (git:1e11d64)".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::InstallStarted {
            app_name: "app".into(),
            bundle_id: "fail.golem.test".into(),
            script_path: "scripts/install-ios.sh".into(),
            target: target.into(),
            os_major: 26,
        },
    ));
    ev.push((
        dev(),
        EventKind::InstallOutput {
            app_name: "app".into(),
            line: "Installing on simulator...".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::InstallFinished {
            app_name: "app".into(),
            bundle_id: "fail.golem.test".into(),
            success: true,
            duration_ms: 46_198,
            exit_code: None,
            error: None,
            code: None,
            target: target.into(),
            os_major: 26,
        },
    ));

    // ── Flow / block / step (+ every substep) ──
    ev.push((
        dev(),
        EventKind::FlowStarted {
            flow_name: "tap.test".into(),
            os_major: 26,
            repeat: None,
        },
    ));
    ev.push((
        dev(),
        EventKind::BlockStarted {
            block_name: "tap_interactions".into(),
            block_index: 0,
            iteration: 0,
        },
    ));
    ev.push((
        dev(),
        EventKind::StepStarted {
            global_step_index: 1,
            block_name: "tap_interactions".into(),
            step_index_in_block: 0,
            action: "tap".into(),
            selector_label: "text=Submit".into(),
        },
    ));
    for s in all_substeps() {
        ev.push((dev(), EventKind::Substep(s)));
    }
    ev.push((
        dev(),
        EventKind::StepFinished {
            global_step_index: 1,
            outcome: StepOutcome::Success,
            duration_ms: 1_147,
            retry_count: 0,
            screenshot_path: Some(".golem/results/tap/ios/screenshots/step-1.png".into()),
            tree_stats: TreeStats {
                fetches: 3,
                min_nodes: 40,
                max_nodes: 46,
            },
        },
    ));
    ev.push((
        dev(),
        EventKind::PerfSnapshot(PerfSnapshotData {
            label: "tap_interactions:iPhone 17:0".into(),
            memory_mb: Some(128.5),
            cpu_percent: Some(12.0),
            threads: Some(24),
            file_descriptors: Some(60),
            disk_mb: Some(2.5),
            net_rx_kb: Some(10.0),
            net_tx_kb: Some(4.0),
            launch_ms: Some(320),
            timestamp: "2026-07-22T09:10:00Z".into(),
        }),
    ));
    ev.push((
        dev(),
        EventKind::A11yAudit {
            audit: A11yAudit {
                label: "tap_interactions:iPhone 17:0".into(),
                issues: vec![A11yIssue {
                    check_id: "touch_target_too_small".into(),
                    severity: Severity::Warning,
                    message: "Button is smaller than the 44dp minimum".into(),
                    element_type: "button".into(),
                    element_label: Some("Submit".into()),
                    element_bounds: Some(rect()),
                    measure_bounds: None,
                    related_bounds: vec![],
                    occlusion: vec![],
                    confidence: 0.8,
                    detail: Some("32dp".into()),
                }],
                screenshot_path: Some(".golem/results/tap/ios/screenshots/a11y-0.png".into()),
            },
        },
    ));
    ev.push((
        dev(),
        EventKind::BlockFinished {
            block_name: "tap_interactions".into(),
            block_index: 0,
            iteration: 0,
            recording_path: Some(".golem/results/tap/ios/recordings/block-0.mp4".into()),
        },
    ));

    // ── Per-outcome steps ──
    ev.extend(outcome_steps());

    // ── Skips / recovery / non-run ──
    ev.push((
        dev(),
        EventKind::FlowSkipped {
            flow_name: "tap.test".into(),
            reason: "coverage satisfied by a peer run".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::DeviceRecovering {
            device_id: IOS.into(),
            reason: "ANR detected".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::DeviceRecovered {
            device_id: IOS.into(),
            duration_ms: 9_300,
            success: true,
            detail: "rebooted in 9300ms · disk 12.4GB free".into(),
        },
    ));
    ev.push((
        dev(),
        EventKind::FlowCouldNotRun {
            flow_name: "checkout.test".into(),
            reason: "install script failed".into(),
            code: FailureCode::FlowExternalFailed,
        },
    ));
    ev.push((
        dev(),
        EventKind::FlowFinished {
            flow_name: "tap.test".into(),
            success: true,
            duration_ms: 36_252,
            seed: 252_697_881_020_433_280,
            os_major: 26,
            code: None,
            repeat: Some(RepeatContext { index: 0, total: 3 }),
        },
    ));

    // ── Suite ──
    ev.push((
        suite(),
        EventKind::SuiteFinished {
            duration_ms: 96_506,
            passed: 1,
            failed: 0,
            skipped: 1,
        },
    ));

    ev
}

#[tokio::main]
async fn main() {
    let (tx, subs) = event_channel();
    let rx = subs.subscribe();
    for (device_id, kind) in gallery() {
        tx.emit(device_id, kind);
    }
    // Drop every sender so the renderer's `recv()` loop sees the channel close
    // and returns instead of hanging. `subs` holds a sender clone too.
    drop(tx);
    drop(subs);
    // verbose = true (stream substeps live), multi_device = false (no device
    // prefix column), debug = true (nothing suppressed).
    stream_human(rx, true, false, true).await;
}

/// Compile-time completeness backstop — see the module docs. No wildcard arm:
/// a new `EventKind` variant must be added here (and to `gallery()`).
#[allow(dead_code)]
fn _eventkind_exhaustiveness_guard(k: &EventKind) {
    match k {
        EventKind::SuiteStarted { .. }
        | EventKind::SuiteFinished { .. }
        | EventKind::SuiteLint { .. }
        | EventKind::SuitePlanned { .. }
        | EventKind::FlowStarted { .. }
        | EventKind::FlowFinished { .. }
        | EventKind::BlockStarted { .. }
        | EventKind::BlockFinished { .. }
        | EventKind::A11yAudit { .. }
        | EventKind::StepStarted { .. }
        | EventKind::StepFinished { .. }
        | EventKind::Substep(_)
        | EventKind::PerfSnapshot(_)
        | EventKind::InstallStarted { .. }
        | EventKind::InstallOutput { .. }
        | EventKind::InstallFinished { .. }
        | EventKind::FlowSkipped { .. }
        | EventKind::DeviceRecovering { .. }
        | EventKind::DeviceRecovered { .. }
        | EventKind::FlowCouldNotRun { .. }
        | EventKind::InstallSkipped { .. }
        | EventKind::InstallCacheMiss { .. }
        | EventKind::FlowParseFailed { .. }
        | EventKind::DeviceAutoBoot { .. }
        | EventKind::DeviceAutoBootFinished { .. }
        | EventKind::SlotSetupFailed { .. }
        | EventKind::ResourcesWaiting { .. }
        | EventKind::CompanionStarting { .. }
        | EventKind::CompanionReady { .. }
        | EventKind::InstallCacheFileBroken { .. }
        | EventKind::InstallCacheWriteFailed { .. }
        | EventKind::DeviceSettingsWarning { .. }
        | EventKind::CompanionRestarting { .. }
        | EventKind::DeviceCleanupWarning { .. }
        | EventKind::DeviceBootRequested { .. }
        | EventKind::RegistrationError { .. }
        | EventKind::RegistrationCompleted { .. } => {}
    }
}

/// Compile-time completeness backstop for substeps — see the module docs.
#[allow(dead_code)]
fn _substep_exhaustiveness_guard(s: &SubstepEvent) {
    match s {
        SubstepEvent::ElementResolved { .. }
        | SubstepEvent::ElementNotFound { .. }
        | SubstepEvent::Tap { .. }
        | SubstepEvent::DoubleTap { .. }
        | SubstepEvent::LongPress { .. }
        | SubstepEvent::TextInput { .. }
        | SubstepEvent::Backspace { .. }
        | SubstepEvent::Swipe { .. }
        | SubstepEvent::ScrollStarted { .. }
        | SubstepEvent::ScrollAttempt { .. }
        | SubstepEvent::ScrollFound { .. }
        | SubstepEvent::ScrollDirectionReversed { .. }
        | SubstepEvent::ScrollStrategySwitch { .. }
        | SubstepEvent::AssertionMatch { .. }
        | SubstepEvent::AssertionMismatch { .. }
        | SubstepEvent::AlertFound { .. }
        | SubstepEvent::RetryAttempt { .. }
        | SubstepEvent::HttpRequest { .. }
        | SubstepEvent::BashCommand { .. }
        | SubstepEvent::PostSettle { .. }
        | SubstepEvent::AppLaunch { .. }
        | SubstepEvent::AppStop { .. }
        | SubstepEvent::DriverWarning { .. }
        | SubstepEvent::Screenshot { .. }
        | SubstepEvent::BarrierAborted { .. } => {}
    }
}
