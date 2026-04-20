// golem-events: structured event stream for test execution
#![deny(clippy::unwrap_used)]

pub mod channel;
pub mod emitter;

use std::time::Instant;

use serde::Serialize;

// ── Geometry ──

/// A point in 2D screen space.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// A rectangle in screen space.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

// ── Identity ──

/// Unique identifier for a device execution (e.g. "ios/iPhone 15 Pro").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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

// ── Step outcome (shared between events and reports) ──

#[derive(Debug, Clone, Serialize)]
pub enum StepOutcome {
    Success,
    Warning(String),
    Failed(String),
    Skipped,
    Ignored,
}

// ── Performance snapshot data ──

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone)]
pub enum EventKind {
    // Suite level
    SuiteStarted { flow_count: usize },
    SuiteFinished { duration_ms: u64, passed: usize, failed: usize },

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
}

// ── Substep events ──

#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Copy, Default, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub enum ScrollAttemptResult {
    PageScrolled,
    InnerScrollableDetected,
    Stall { count: u32, max: u32 },
    BoundaryReached,
}
