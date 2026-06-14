use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use golem_devices::DeviceInfo;
use golem_events::emitter::DeviceEmitter;
use golem_events::{EventKind, SubstepEvent, TreeStats};
use rand_chacha::ChaCha8Rng;

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
    /// Tree fetch statistics for the current step (reset between steps).
    pub step_tree_stats: Mutex<TreeStats>,
    /// Seeded RNG for deterministic fake data generation.
    pub rng: Mutex<ChaCha8Rng>,
    /// Resolved record-default visible to blocks in the current flow.
    /// Computed by `execute_flow` at entry as
    /// `flow.options.record.or(parent_default).or(project_record)`,
    /// then combined per block with `capture_config.cli_force_record`
    /// (overrides) and `block.record` (explicit per-block).
    pub inherited_record_default: bool,
}

impl ExecutionContext<'_> {
    /// Record a launch timing measurement.
    pub fn set_launch_ms(&self, ms: u64) {
        self.last_launch_ms.store(ms, Ordering::Relaxed);
    }

    /// Take the last launch timing (resets to 0).
    pub fn take_launch_ms(&self) -> Option<u64> {
        let val = self.last_launch_ms.swap(0, Ordering::Relaxed);
        if val > 0 { Some(val) } else { None }
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
}

#[cfg(test)]
pub fn test_ctx(tmp: &std::path::Path) -> ExecutionContext<'_> {
    use rand::SeedableRng;
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
        step_tree_stats: Mutex::new(TreeStats::default()),
        rng: Mutex::new(ChaCha8Rng::from_entropy()),
        inherited_record_default: false,
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
        use rand::SeedableRng;
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
            step_tree_stats: Mutex::new(TreeStats::default()),
            rng: Mutex::new(ChaCha8Rng::from_entropy()),
            inherited_record_default: false,
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

    // 8. take_tree_stats drains the accumulated stats (reset between steps).
    #[test]
    fn take_tree_stats_resets_accumulator() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let ctx = test_ctx(tmp.path());
        ctx.record_tree_fetch(5);
        let first = ctx.take_tree_stats();
        assert_eq!(first.fetches, 1, "first take SHALL report the recorded fetch");
        let second = ctx.take_tree_stats();
        assert_eq!(
            second.fetches, 0,
            "stats SHALL be reset after take"
        );
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
