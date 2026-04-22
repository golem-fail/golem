// golem-events: structured event stream for test execution
#![deny(clippy::unwrap_used)]

pub mod channel;
pub mod emitter;

use std::time::Instant;

use serde::{Deserialize, Serialize};

// ── Geometry ──

/// A point in 2D screen space.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// A rectangle in screen space.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

// ── Identity ──

/// Unique identifier for a device execution.
///
/// Canonical format: `{platform}/{device.name}` — e.g. `ios/iPhone 15 Pro`
/// or `android/Pixel_7_API_34`. Suite-level events (`SuitePlanned`,
/// `SuiteStarted`, `SuiteFinished`) use the sentinel `"suite"`.
///
/// Both pre-install (`InstallStarted` / `InstallFinished`) and per-flow
/// events share this scheme so downstream renderers can slot events into
/// the same device column without a platform/device lookup. Consumers that
/// display a human-readable device label render the string verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Event envelope ──

/// Top-level event. Every event carries device identity and timing.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub device_id: DeviceId,
    pub timestamp: Instant,
    pub kind: EventKind,
}

/// Wire-format event for serialization over sockets/IPC.
/// Same as `Event` but without `Instant` (not serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEvent {
    pub seq: u64,
    pub device_id: DeviceId,
    pub kind: EventKind,
}

impl From<&Event> for WireEvent {
    fn from(e: &Event) -> Self {
        Self {
            seq: e.seq,
            device_id: e.device_id.clone(),
            kind: e.kind.clone(),
        }
    }
}

impl WireEvent {
    /// Convert back to a full `Event` with `Instant::now()` as timestamp.
    pub fn into_event(self) -> Event {
        Event {
            seq: self.seq,
            device_id: self.device_id,
            timestamp: Instant::now(),
            kind: self.kind,
        }
    }
}

// ── Step outcome (shared between events and reports) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepOutcome {
    Success,
    Warning(String),
    Failed(String),
    Skipped,
    Ignored,
}

// ── Performance snapshot data ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfSnapshotData {
    pub label: String,
    pub memory_mb: Option<f64>,
    pub cpu_percent: Option<f64>,
    pub threads: Option<u32>,
    pub file_descriptors: Option<u32>,
    pub disk_mb: Option<f64>,
    pub net_rx_kb: Option<f64>,
    pub net_tx_kb: Option<f64>,
    pub launch_ms: Option<u64>,
    pub timestamp: String,
}

// ── Event hierarchy ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    // Suite level
    SuiteStarted { flow_count: usize },
    SuiteFinished { duration_ms: u64, passed: usize, failed: usize },

    /// Diagnostic snapshot of the Plan phase output. Emitted once per suite
    /// when `--verbose` is on. All fields are pre-formatted `String`s:
    /// `stream_human` prints them verbatim and the orchestrator forwarder
    /// relays them unchanged. Other sinks (accumulator, JSON, TOON, JUnit)
    /// intentionally ignore this variant — it carries no structured data.
    ///
    /// If a machine-readable plan payload is ever needed (programmatic
    /// consumers, report tooling), add a separate structured sibling event
    /// rather than extending this one; its contract is "UI-only strings".
    SuitePlanned {
        /// One line per FlowRun, e.g. `#1 tap.test: ios/v18 apps=[app]`.
        flow_runs: Vec<String>,
        /// One line per InstallEntry, e.g. `ios app → fail.golem.test`.
        install_entries: Vec<String>,
        /// Per-slot device availability against the plan-time snapshot.
        /// Each line: `<slot-shape> — <total> matches (<booted> booted,
        /// <shutdown> shutdown)`. De-duplicated by shape. Lets the user
        /// see up front whether the suite's device needs can be satisfied
        /// and how many parallel runs are feasible per requirement.
        device_availability: Vec<String>,
    },

    // Flow level
    FlowStarted { flow_name: String },
    FlowFinished { flow_name: String, success: bool, duration_ms: u64, seed: u64 },

    // Block level
    BlockStarted { block_name: String, block_index: usize, iteration: u32 },
    BlockFinished { block_name: String, block_index: usize },

    // Step level
    StepStarted {
        global_step_index: u64,
        block_name: String,
        step_index_in_block: usize,
        action: String,
        selector_label: String,
    },
    StepFinished {
        global_step_index: u64,
        outcome: StepOutcome,
        duration_ms: u64,
        retry_count: u32,
        screenshot_path: Option<String>,
        tree_stats: TreeStats,
    },

    // Substep detail
    Substep(SubstepEvent),

    // Performance
    PerfSnapshot(PerfSnapshotData),

    // Install script (app install before flow starts)
    InstallStarted {
        app_name: String,
        bundle_id: String,
        script_path: String,
        /// Pre-formatted target: `iPhone 16e (ios/v18/phone)`.
        target: String,
    },
    InstallOutput {
        app_name: String,
        /// A single line of the script's stderr.
        line: String,
    },
    InstallFinished {
        app_name: String,
        bundle_id: String,
        success: bool,
        duration_ms: u64,
        /// Exit code when nonzero, or `None` on success/timeout.
        exit_code: Option<i32>,
        /// Error detail when failed (timeout reason, exit code, or tail of stderr).
        error: Option<String>,
        /// Pre-formatted target (same as InstallStarted.target).
        target: String,
    },
    /// A flow was skipped on a device because a prior install failed.
    FlowSkipped {
        flow_name: String,
        reason: String,
    },
}

// ── Substep events ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubstepEvent {
    // Element resolution
    ElementResolved {
        selector: String,
        bounds: Rect,
        tap_point: Point,
    },
    ElementNotFound {
        selector: String,
        timeout_ms: u64,
    },

    // Interactions
    Tap {
        point: Point,
        element_bounds: Option<Rect>,
    },
    DoubleTap {
        point: Point,
        element_bounds: Option<Rect>,
    },
    LongPress {
        point: Point,
        duration_ms: u64,
        element_bounds: Option<Rect>,
    },
    TextInput {
        text: String,
        field_bounds: Option<Rect>,
    },
    Backspace {
        count: u32,
    },
    Swipe {
        from: Point,
        to: Point,
        duration_ms: Option<u64>,
    },

    // Scroll (richest substep source)
    ScrollStarted {
        selector: String,
        direction: String,
    },
    ScrollAttempt {
        attempt: u32,
        direction: String,
        strategy_index: usize,
        from: Point,
        to: Point,
        result: ScrollAttemptResult,
        tree_stats: TreeStats,
    },
    ScrollFound {
        selector: String,
        position: Point,
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

    // Assertions
    AssertionMatch {
        expected: String,
        actual: String,
        element_bounds: Option<Rect>,
    },
    AssertionMismatch {
        expected: String,
        actual: Option<String>,
    },
    AlertFound {
        text: Option<String>,
    },

    // Retry
    RetryAttempt {
        attempt: u32,
        max: u32,
        delay_ms: u64,
        error: String,
    },

    // External
    HttpRequest {
        method: String,
        url: String,
        status: Option<u16>,
        duration_ms: u64,
    },
    BashCommand {
        command: String,
        exit_code: Option<i32>,
        duration_ms: u64,
    },

    // App lifecycle
    AppLaunch {
        bundle: String,
        duration_ms: u64,
    },
    AppStop {
        bundle: String,
    },

    // Media
    Screenshot {
        path: String,
    },

    // Device
    DeviceRotation {
        orientation: String,
    },

    // Barrier
    BarrierAborted {
        step_count: u64,
    },
}

/// Tree fetch statistics for a single operation (step or scroll iteration).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TreeStats {
    pub fetches: u32,
    pub min_nodes: u32,
    pub max_nodes: u32,
}

impl TreeStats {
    pub fn record(&mut self, node_count: u32) {
        self.fetches += 1;
        if self.fetches == 1 {
            self.min_nodes = node_count;
            self.max_nodes = node_count;
        } else {
            self.min_nodes = self.min_nodes.min(node_count);
            self.max_nodes = self.max_nodes.max(node_count);
        }
    }

    pub fn merge(&mut self, other: &TreeStats) {
        if other.fetches == 0 { return; }
        self.fetches += other.fetches;
        if self.min_nodes == 0 || other.min_nodes < self.min_nodes {
            self.min_nodes = other.min_nodes;
        }
        self.max_nodes = self.max_nodes.max(other.max_nodes);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScrollAttemptResult {
    PageScrolled,
    InnerScrollableDetected,
    Stall { count: u32, max: u32 },
    BoundaryReached,
}
