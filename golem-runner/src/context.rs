use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use golem_devices::DeviceInfo;
use golem_events::emitter::DeviceEmitter;
use golem_events::{EventKind, SubstepEvent, TreeStats};

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
        device: None,
        perf_collector: None,
        last_launch_ms: AtomicU64::new(0),
        emitter: None,
        step_tree_stats: Mutex::new(TreeStats::default()),
    }
}
