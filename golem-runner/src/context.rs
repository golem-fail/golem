use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use golem_devices::DeviceInfo;
use golem_element::Element;
use golem_events::emitter::DeviceEmitter;
use golem_events::{EventKind, SubstepEvent, TreeStats};
use golem_vars::seed::FakeRng;

use crate::capture::CaptureConfig;
use crate::perf::PerfCollectorSet;

/// Flow-level context threaded through the execution pipeline.
pub struct ExecutionContext<'a> {
    pub flow_dir: &'a Path,
    pub project_root: &'a Path,
    pub capture_config: &'a CaptureConfig,
    pub flow_name: &'a str,
    pub block_name: Option<&'a str>,
    pub step_index: usize,
    /// Global step counter across all blocks (for screenshot filenames).
    pub global_step_index: u64,
    /// Current block iteration (for data-driven/loop blocks).
    pub block_iteration: u32,
    /// The device running this flow. Used for block-level `where` filtering.
    pub device: Option<&'a DeviceInfo>,
    /// Performance collector set — `None` when perf is disabled.
    pub perf_collector: Option<&'a PerfCollectorSet>,
    /// Last launch duration in ms, shared between action handlers and executor.
    pub last_launch_ms: AtomicU64,
    /// Event emitter for structured test output.
    pub emitter: Option<&'a DeviceEmitter>,
    /// Resolved accessibility audit level for this flow. `Off` disables the
    /// block-end audit (the default in non-suite contexts, e.g. unit tests);
    /// the suite resolves it from CLI `--a11y` / `[flow.options].a11y`.
    pub a11y_level: crate::accessibility::A11yLevel,
    /// CLI `--a11y-min-confidence` override. When set it wins over
    /// `[flow.options].a11y_min_confidence` and the level default, so a run can
    /// surface or suppress heuristic findings without editing the flow.
    pub a11y_min_confidence: Option<f32>,
    /// Tree fetch statistics for the current step (reset between steps).
    pub step_tree_stats: Mutex<TreeStats>,
    /// The last settled UI tree captured by the post-step settle, tagged with
    /// the `global_step_index` at capture. The block-end a11y audit reuses it
    /// (instead of a fresh `/hierarchy` fetch) when the block's last step
    /// settled. Per-`ctx`, so safe across concurrent flows — unlike the global
    /// `TreeStats` statics. The settle's tree is otherwise dropped, so storing
    /// it is a move, not a clone.
    pub last_settled_tree: Mutex<Option<(Element, u64, Instant)>>,
    /// A coherent `(tree, screenshot PNG)` pair already captured this step —
    /// from a `--trace` boundary, or the failure capture — tagged with the
    /// step's `global_step_index`. The a11y audit reuses it (any level — even
    /// strict skips its own capture) instead of re-shooting: the block-end audit
    /// consumes the last step's pair, the failure handler the failing step's.
    /// `None` when nothing was captured this step.
    pub trace_pair: Mutex<Option<(Element, Vec<u8>, u64)>>,
    /// Seeded RNG for deterministic fake data generation, carrying the run's
    /// date anchor (see [`golem_vars::seed::FakeRng`]).
    pub rng: Mutex<FakeRng>,
    /// Resolved record-default visible to blocks in the current flow.
    /// Computed by `execute_flow` at entry as
    /// `flow.options.record.or(parent_default).or(project_record)`,
    /// then combined per block with `capture_config.cli_force_record`
    /// (overrides) and `block.record` (explicit per-block).
    pub inherited_record_default: bool,
    /// One-shot hint: when a `/type` or `/backspace` companion couldn't
    /// confirm the field text changed (slow IME may not have propagated
    /// yet), the action handler sets this so the *next* post-step settle
    /// runs on an extended budget. Consumed (swapped back to false) by
    /// that settle, so it only stretches the one settle immediately
    /// following the un-verified mutation.
    pub extend_next_settle: AtomicBool,
}

impl ExecutionContext<'_> {
    /// Record a launch timing measurement.
    pub fn set_launch_ms(&self, ms: u64) {
        self.last_launch_ms.store(ms, Ordering::Relaxed);
    }

    /// Take the last launch timing (resets to 0).
    pub fn take_launch_ms(&self) -> Option<u64> {
        let val = self.last_launch_ms.swap(0, Ordering::Relaxed);
        if val > 0 {
            Some(val)
        } else {
            None
        }
    }

    /// Emit a top-level event (step started, flow finished, etc.).
    pub fn emit(&self, kind: EventKind) {
        if let Some(e) = self.emitter {
            e.emit(kind);
        }
    }

    /// Emit a substep detail event.
    pub fn substep(&self, event: SubstepEvent) {
        if let Some(e) = self.emitter {
            e.substep(event);
        }
    }

    /// Record a tree fetch (called by wait_for_settle).
    pub fn record_tree_fetch(&self, node_count: u32) {
        if let Ok(mut stats) = self.step_tree_stats.lock() {
            stats.record(node_count);
        }
    }

    /// Take and reset the step-level tree stats.
    pub fn take_tree_stats(&self) -> TreeStats {
        if let Ok(mut stats) = self.step_tree_stats.lock() {
            std::mem::take(&mut *stats)
        } else {
            TreeStats::default()
        }
    }

    /// Cache the settled tree from a post-step settle, tagged with the step's
    /// `global_step_index`. The tree is moved in (the settle would otherwise
    /// drop it), overwriting any prior step's cached tree.
    pub fn cache_settled_tree(&self, tree: Element, step_index: u64) {
        if let Ok(mut slot) = self.last_settled_tree.lock() {
            // Stamp the capture instant so the recording-frame a11y path can pull
            // the video frame at the exact moment this tree was read.
            *slot = Some((tree, step_index, Instant::now()));
        }
    }

    /// Take the most recent settled tree cached this block (paired with its
    /// capture instant), or `None` if no step settled. Non-strict on which step
    /// it came from — for a11y, auditing a near-end settled state is fine and
    /// exact block-end timing isn't the goal; `clear_settled_tree` at block
    /// start guarantees it's never a previous block's. Always clears the slot.
    pub fn take_latest_settled_tree(&self) -> Option<(Element, Instant)> {
        if let Ok(mut slot) = self.last_settled_tree.lock() {
            slot.take().map(|(tree, _idx, at)| (tree, at))
        } else {
            None
        }
    }

    /// Clear any cached settled tree. Called at block start so the block-end
    /// audit's [`take_latest_settled_tree`] never returns a *previous* block's
    /// tree (the cache is per-`ctx` and persists across blocks otherwise).
    pub fn clear_settled_tree(&self) {
        if let Ok(mut slot) = self.last_settled_tree.lock() {
            *slot = None;
        }
    }

    /// Cache the coherent `(tree, screenshot PNG)` pair from a `--trace`
    /// boundary capture, tagged with the step's `global_step_index`. Overwrites
    /// any prior step's pair (only the latest — the block end — is reused).
    pub fn cache_trace_pair(&self, tree: Element, shot_png: Vec<u8>, step_index: u64) {
        if let Ok(mut slot) = self.trace_pair.lock() {
            *slot = Some((tree, shot_png, step_index));
        }
    }

    /// Take the cached trace pair iff it was captured at `expect_step_index`
    /// (the block's last-executed step), so the audit reuses an aligned
    /// `(tree, shot)` rather than re-capturing. `None` otherwise. Clears the slot.
    pub fn take_trace_pair_at(&self, expect_step_index: u64) -> Option<(Element, Vec<u8>)> {
        if let Ok(mut slot) = self.trace_pair.lock() {
            match slot.take() {
                Some((tree, shot, idx)) if idx == expect_step_index => Some((tree, shot)),
                _ => None,
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
pub fn test_ctx(tmp: &std::path::Path) -> ExecutionContext<'_> {
    use std::sync::LazyLock;
    static DEFAULT_CAPTURE: LazyLock<CaptureConfig> = LazyLock::new(|| CaptureConfig {
        screenshot_on_failure: false,
        ..CaptureConfig::default()
    });
    ExecutionContext {
        flow_dir: tmp,
        project_root: tmp,
        capture_config: &DEFAULT_CAPTURE,
        flow_name: "test",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: AtomicU64::new(0),
        emitter: None,
        a11y_level: crate::accessibility::A11yLevel::Off,
        a11y_min_confidence: None,
        step_tree_stats: Mutex::new(TreeStats::default()),
        last_settled_tree: Mutex::new(None),
        trace_pair: Mutex::new(None),
        rng: Mutex::new(FakeRng::from_optional_seed(None)),
        inherited_record_default: false,
        extend_next_settle: AtomicBool::new(false),
    }
}

/// Test-only harness owning the values an [`ExecutionContext`] borrows, so
/// perf/lifecycle tests can inject a `PerfCollectorSet` (no device I/O) and a
/// capturing emitter, then assert on the events the context emits.
///
/// Usage:
/// ```ignore
/// let h = TestHarness::new(tmp.path(), &[("com.example.app".into(), raw)]);
/// let ctx = h.ctx();
/// ctx.emit(EventKind::SuiteStarted { flow_count: 1 });
/// let event = h.recv().await;
/// ```
#[cfg(test)]
pub struct TestHarness {
    tmp: std::path::PathBuf,
    capture_config: CaptureConfig,
    perf: crate::perf::PerfCollectorSet,
    emitter: DeviceEmitter,
    rx: tokio::sync::broadcast::Receiver<golem_events::Event>,
    // Keep subscriptions alive so the broadcast channel is not closed.
    _subs: golem_events::channel::EventSubscriptions,
}

#[cfg(test)]
impl TestHarness {
    /// Build a harness whose perf collector yields the supplied raw data per
    /// bundle (first entry active) and whose emitter feeds a captured receiver.
    pub fn new(tmp: &std::path::Path, apps: &[(String, crate::perf::RawPerfData)]) -> Self {
        use golem_events::channel::event_channel;
        use golem_events::DeviceId;
        let (sender, subs) = event_channel();
        let rx = subs.subscribe();
        Self {
            tmp: tmp.to_path_buf(),
            capture_config: CaptureConfig {
                screenshot_on_failure: false,
                ..CaptureConfig::default()
            },
            perf: crate::perf::PerfCollectorSet::from_raw(apps),
            emitter: DeviceEmitter::new(sender, DeviceId("test-device".into())),
            rx,
            _subs: subs,
        }
    }

    /// Build an [`ExecutionContext`] borrowing this harness's owned values, with
    /// the injected perf collector and capturing emitter wired in.
    pub fn ctx(&self) -> ExecutionContext<'_> {
        ExecutionContext {
            flow_dir: &self.tmp,
            project_root: &self.tmp,
            capture_config: &self.capture_config,
            flow_name: "test",
            block_name: None,
            step_index: 0,
            global_step_index: 0,
            block_iteration: 0,
            device: None,
            perf_collector: Some(&self.perf),
            last_launch_ms: AtomicU64::new(0),
            emitter: Some(&self.emitter),
            a11y_level: crate::accessibility::A11yLevel::Off,
            a11y_min_confidence: None,
            step_tree_stats: Mutex::new(TreeStats::default()),
            last_settled_tree: Mutex::new(None),
            trace_pair: Mutex::new(None),
            rng: Mutex::new(FakeRng::from_optional_seed(None)),
            inherited_record_default: false,
            extend_next_settle: AtomicBool::new(false),
        }
    }

    /// Receive the next captured event (non-blocking try).
    pub fn try_recv(&mut self) -> Option<golem_events::Event> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. take_launch_ms returns None when nothing was recorded (initial 0).
    #[test]
    fn take_launch_ms_none_when_unset() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        assert_eq!(
            ctx.take_launch_ms(),
            None,
            "unrecorded launch SHALL yield None"
        );
    }

    // 2. set_launch_ms then take returns the stored value once.
    #[test]
    fn set_then_take_launch_ms_returns_value() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.set_launch_ms(42);
        assert_eq!(
            ctx.take_launch_ms(),
            Some(42),
            "recorded launch SHALL be returned"
        );
    }

    // 3. take_launch_ms resets to 0 — a second take yields None.
    #[test]
    fn take_launch_ms_resets_after_take() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.set_launch_ms(7);
        let _ = ctx.take_launch_ms();
        assert_eq!(
            ctx.take_launch_ms(),
            None,
            "second take SHALL yield None after reset"
        );
    }

    // 4. set_launch_ms overwrites a previous unread value (last writer wins).
    #[test]
    fn set_launch_ms_overwrites_previous() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.set_launch_ms(10);
        ctx.set_launch_ms(20);
        assert_eq!(
            ctx.take_launch_ms(),
            Some(20),
            "latest set SHALL overwrite the prior value"
        );
    }

    // 5. set_launch_ms(0) is treated as "no measurement" (boundary: 0 -> None).
    #[test]
    fn set_launch_ms_zero_is_none() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.set_launch_ms(0);
        assert_eq!(
            ctx.take_launch_ms(),
            None,
            "zero launch ms SHALL be treated as no measurement"
        );
    }

    // 6. take_tree_stats on a fresh context returns the default (no fetches).
    #[test]
    fn take_tree_stats_default_when_empty() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        let stats = ctx.take_tree_stats();
        assert_eq!(stats.fetches, 0, "fresh context SHALL have no fetches");
        assert_eq!(stats.min_nodes, 0, "fresh context SHALL have zero min");
        assert_eq!(stats.max_nodes, 0, "fresh context SHALL have zero max");
    }

    // 7. record_tree_fetch forwards each node_count through the mutex and
    //    accumulates into one shared TreeStats (the part context.rs owns; the
    //    min/max ordering algorithm itself is covered in golem-events).
    #[test]
    fn record_tree_fetch_accumulates_forwarded_counts() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        // 7a. Three separate calls SHALL all land in the same locked accumulator.
        ctx.record_tree_fetch(30);
        ctx.record_tree_fetch(10);
        ctx.record_tree_fetch(50);
        let stats = ctx.take_tree_stats();
        assert_eq!(
            stats.fetches, 3,
            "each record_tree_fetch call SHALL increment the shared count"
        );
        // 7b. The smallest and largest distinct values are only reachable if the
        //     exact node_count was forwarded faithfully (not dropped/altered).
        assert_eq!(
            stats.min_nodes, 10,
            "smallest node_count SHALL be forwarded intact to stats"
        );
        assert_eq!(
            stats.max_nodes, 50,
            "largest node_count SHALL be forwarded intact to stats"
        );
    }

    fn tiny_tree() -> Element {
        Element {
            element_type: "Root".into(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: false,
            focused: false,
            bounds: golem_element::Bounds::new(0, 0, 100, 100),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: vec![],
        }
    }

    // 7b. Settled-tree cache: the latest settled tree (any step) is returned,
    //     cleared once on take, and cleared at block start.
    #[test]
    fn latest_settled_tree_returned_and_taken_once() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.cache_settled_tree(tiny_tree(), 3);
        ctx.cache_settled_tree(tiny_tree(), 5); // later step overwrites
        assert!(
            ctx.take_latest_settled_tree().is_some(),
            "the latest cached settled tree SHALL be returned"
        );
        assert!(
            ctx.take_latest_settled_tree().is_none(),
            "the slot SHALL be cleared after taking"
        );
    }

    #[test]
    fn clear_settled_tree_drops_cache() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.cache_settled_tree(tiny_tree(), 1);
        ctx.clear_settled_tree();
        assert!(
            ctx.take_latest_settled_tree().is_none(),
            "clear SHALL drop a prior block's cached tree"
        );
    }

    // 7c. Trace-pair cache: same matching/stale/take-once contract as the
    //     settled tree — the block-end audit reuses the last step's pair only.
    #[test]
    fn trace_pair_reused_on_matching_index_with_shot() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.cache_trace_pair(tiny_tree(), vec![1, 2, 3], 5);
        let pair = ctx.take_trace_pair_at(5);
        assert!(pair.is_some(), "matching index SHALL reuse the pair");
        assert_eq!(
            pair.expect("pair").1,
            vec![1, 2, 3],
            "the cached screenshot bytes SHALL come back intact"
        );
    }

    #[test]
    fn trace_pair_skipped_on_stale_index() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.cache_trace_pair(tiny_tree(), vec![0], 3);
        assert!(
            ctx.take_trace_pair_at(7).is_none(),
            "stale pair (earlier step) SHALL NOT be reused"
        );
    }

    #[test]
    fn trace_pair_taken_once() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.cache_trace_pair(tiny_tree(), vec![9], 1);
        assert!(ctx.take_trace_pair_at(1).is_some());
        assert!(
            ctx.take_trace_pair_at(1).is_none(),
            "the slot SHALL be cleared after taking"
        );
    }

    // 8. take_tree_stats drains the accumulated stats (reset between steps).
    #[test]
    fn take_tree_stats_resets_accumulator() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.record_tree_fetch(5);
        let first = ctx.take_tree_stats();
        assert_eq!(
            first.fetches, 1,
            "first take SHALL report the recorded fetch"
        );
        let second = ctx.take_tree_stats();
        assert_eq!(second.fetches, 0, "stats SHALL be reset after take");
    }

    // 9. TestHarness seam: the injected perf collector yields the supplied raw
    //    data (no device I/O) and the capturing emitter records emitted events.
    #[tokio::test]
    async fn test_harness_injects_perf_and_captures_events() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = crate::perf::RawPerfData {
            memory_mb: Some(123.0),
            ..crate::perf::RawPerfData::default()
        };
        let mut harness = TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        {
            let ctx = harness.ctx();

            // 9a. The injected collector is wired in and returns the supplied data.
            let collector = ctx
                .perf_collector
                .expect("TestHarness SHALL wire in a perf collector");
            assert_eq!(
                collector.capture().await.memory_mb,
                Some(123.0),
                "injected collector SHALL yield the supplied raw data without device I/O"
            );

            // 9b. Events emitted through the context are captured by the harness.
            ctx.emit(EventKind::SuiteStarted { flow_count: 1 });
        }
        let event = harness
            .try_recv()
            .expect("capturing emitter SHALL record the emitted event");
        assert!(
            matches!(event.kind, EventKind::SuiteStarted { flow_count: 1 }),
            "captured event SHALL match the emitted EventKind"
        );
    }
}
