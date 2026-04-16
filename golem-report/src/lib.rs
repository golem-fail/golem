// golem-report: test reporting
#![deny(clippy::unwrap_used)]

pub mod human;
pub mod json;
pub mod junit;
pub mod output;
pub mod toon;

/// Result of a single step within a flow.
pub struct StepReport {
    /// The action performed (e.g. "tap", "type", "assert_visible").
    pub action: String,
    /// The target element text or identifier.
    pub target: String,
    /// The outcome of this step.
    pub outcome: StepOutcome,
    /// How long this step took, in milliseconds.
    pub duration_ms: u64,
}

/// Possible outcomes for a single step.
pub enum StepOutcome {
    /// Step completed successfully.
    Success,
    /// Step completed with a warning.
    Warning(String),
    /// Step failed with an error message.
    Failed(String),
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
pub struct FlowReport {
    /// Name of the flow (e.g. "login_flow").
    pub flow_name: String,
    /// Whether the flow passed overall.
    pub success: bool,
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
    /// Performance snapshots captured at block boundaries.
    pub perf_snapshots: Vec<PerfSnapshot>,
}

/// Result of an entire test suite (multiple flows).
pub struct SuiteReport {
    /// Individual flow results.
    pub flows: Vec<FlowReport>,
    /// Total wall-clock duration in milliseconds.
    pub total_duration_ms: u64,
}
