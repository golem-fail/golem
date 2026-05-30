// golem-events: structured event stream for test execution
#![deny(clippy::unwrap_used)]

pub mod channel;
pub mod emitter;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

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

/// `--repeat` context attached to FlowStarted/FlowFinished. `index`
/// is 0-based; `total` is the value passed to `--repeat` (1..=100).
/// `Option<RepeatContext>` is `None` at the default `--repeat 1` so
/// existing logs/events are unchanged.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepeatContext {
    pub index: u32,
    pub total: u32,
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Event envelope ──

/// Top-level event. Every event carries device identity and timing.
///
/// Two clocks on purpose:
/// - `timestamp: Instant` — monotonic, for durations between events. Not
///   serializable; only meaningful inside the emitting process.
/// - `wall_time: SystemTime` — wall-clock, for display (`HH:MM:SS.mmm`),
///   JSON ISO-8601, and TOON unix epoch. Not monotonic — may jump if the
///   system clock moves. Use `timestamp` for intervals, `wall_time` for
///   human-readable display.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub device_id: DeviceId,
    pub timestamp: Instant,
    pub wall_time: SystemTime,
    pub kind: EventKind,
}

/// Wire-format event for serialization over sockets/IPC.
/// Same as `Event` but without `Instant` (not serializable). `wall_time`
/// crosses the wire as unix-epoch nanoseconds so the orchestrator client
/// renders the same time the server saw.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEvent {
    pub seq: u64,
    pub device_id: DeviceId,
    pub wall_time_unix_nanos: u128,
    pub kind: EventKind,
}

impl From<&Event> for WireEvent {
    fn from(e: &Event) -> Self {
        Self {
            seq: e.seq,
            device_id: e.device_id.clone(),
            wall_time_unix_nanos: system_time_to_unix_nanos(e.wall_time),
            kind: e.kind.clone(),
        }
    }
}

impl WireEvent {
    /// Rehydrate into an `Event`. `timestamp` gets a fresh `Instant::now()`
    /// since the sender's monotonic clock isn't meaningful to us; `wall_time`
    /// is reconstructed from the unix-nanos wire value.
    pub fn into_event(self) -> Event {
        Event {
            seq: self.seq,
            device_id: self.device_id,
            timestamp: Instant::now(),
            wall_time: unix_nanos_to_system_time(self.wall_time_unix_nanos),
            kind: self.kind,
        }
    }
}

fn system_time_to_unix_nanos(t: SystemTime) -> u128 {
    // Pre-epoch timestamps shouldn't occur in practice; saturate to 0 so
    // we don't propagate a panic out of a serializer.
    t.duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn unix_nanos_to_system_time(nanos: u128) -> SystemTime {
    // Split into (secs, subsec-nanos) to avoid u128→u64 truncation that
    // `Duration::from_nanos` would cause post-2554.
    let secs = (nanos / 1_000_000_000) as u64;
    let subsec_nanos = (nanos % 1_000_000_000) as u32;
    UNIX_EPOCH + std::time::Duration::new(secs, subsec_nanos)
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
    SuiteFinished { duration_ms: u64, passed: usize, failed: usize, skipped: usize },

    /// Diagnostic snapshot of the Plan phase output. Emitted once per suite
    /// when `--verbose` is on. All fields are pre-formatted `String`s:
    /// `stream_human` prints them verbatim and the orchestrator forwarder
    /// relays them unchanged. Other sinks (accumulator, JSON, TOON, JUnit)
    /// intentionally ignore this variant — it carries no structured data.
    ///
    /// If a machine-readable plan payload is ever needed (programmatic
    /// consumers, report tooling), add a separate structured sibling event
    /// rather than extending this one; its contract is "UI-only strings".
    /// Plan-phase lint findings — warnings about TOML that parses fine
    /// but won't behave as the author probably expects (e.g. `within`
    /// set on an action that doesn't consume it). Pre-formatted, one
    /// string per finding. Emitted once per suite before
    /// `SuitePlanned`; empty `warnings` means none, in which case the
    /// orchestrator omits the event entirely. UI-only — JSON/JUnit
    /// sinks intentionally ignore this variant.
    SuiteLint {
        warnings: Vec<String>,
    },

    SuitePlanned {
        /// One line per FlowRun, e.g. `#1 tap.test: ios/v18 apps=[app]`.
        flow_runs: Vec<String>,
        /// One line per InstallEntry, e.g. `ios app → fail.golem.test`.
        install_entries: Vec<String>,
        /// Per-slot device availability against the plan-time snapshot.
        /// Each line: `<slot-shape> — <n> device(s) (<booted> booted[,
        /// <shutdown> shutdown][, <physical> physical])`. De-duplicated by
        /// shape. The `<n>` is *eligible* devices, not parallel-usable
        /// capacity — shutdown sims must boot first and physical devices
        /// are single-user. See `compute_device_availability` in
        /// `golem-orchestrator` for the semantics note.
        device_availability: Vec<String>,
    },

    // Flow level
    FlowStarted {
        flow_name: String,
        os_major: u32,
        /// `--repeat` context. `None` when N=1 (historical layout
        /// unchanged); `Some((index, total))` with 0-based index when
        /// the suite was fanned out across multiple runs. Carried on
        /// FlowStarted/FlowFinished only — Block/Step events inherit
        /// via the surrounding flow, so renderers and accumulators
        /// partition by (device_id, repeat_index) without bloating
        /// every event.
        repeat: Option<RepeatContext>,
    },
    FlowFinished {
        flow_name: String,
        success: bool,
        duration_ms: u64,
        seed: u64,
        os_major: u32,
        repeat: Option<RepeatContext>,
    },

    // Block level
    BlockStarted { block_name: String, block_index: usize, iteration: u32 },
    BlockFinished {
        block_name: String,
        block_index: usize,
        iteration: u32,
        /// Path to the screen recording for this block iteration, when
        /// recording was active. `None` when recording was off or the
        /// driver could not produce a file (e.g. iOS pre-recordVideo
        /// wiring still bails). Absolute or output-dir-relative — the
        /// renderer decides how to display.
        recording_path: Option<String>,
    },

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
        /// OS major version of the target device (e.g. 18, 26, 34). Plumbed
        /// to downstream reports + TOON/JSON/JUnit so consumers can
        /// distinguish ios/v26 from ios/v18 without parsing `target`.
        os_major: u32,
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
        /// OS major version (same as InstallStarted.os_major).
        os_major: u32,
    },
    /// A flow was skipped on a device because a prior install failed.
    FlowSkipped {
        flow_name: String,
        reason: String,
    },
    /// The persistent install cache decided no install was needed for
    /// this `(device, bundle)`. Emitted once per skipped install. Lets
    /// stream consumers tell the user why no `[install]` lines appeared.
    InstallSkipped {
        app_name: String,
        bundle_id: String,
        /// Same target string format as `InstallStarted.target`.
        target: String,
        /// Human-readable reason: e.g. `cache hit (git:abc1234)` or
        /// `no-build: bundle present on device`.
        reason: String,
    },
    /// The persistent install cache rejected a candidate hit. Emitted
    /// before `InstallStarted` so the user sees *why* a build was
    /// triggered. Only fires when there was a meaningful gate to fail
    /// against (i.e. an entry existed); fresh entries don't emit this.
    InstallCacheMiss {
        app_name: String,
        bundle_id: String,
        target: String,
        /// Specific gate that failed, e.g.
        /// `source fingerprint changed (git:abc → git:def)` or
        /// `device install-time differs (... — external reinstall?)`.
        reason: String,
    },

    // ── Setup narrative ──
    //
    // These cover the pre-flow diagnostic strings the scheduler and
    // per-slot setup used to write directly to stderr. Emitting them
    // as events lets the orchestrator forwarder relay them to remote
    // clients instead of swallowing them on the server terminal.
    //
    // Device identity on each variant: suite-level ones (Parse, AutoBoot,
    // SlotSetupFailed) use the sentinel `"suite"`; device-tied ones
    // (CompanionStarting, CompanionReady, ResourcesWaiting) use the
    // standard `{platform}/{device.name}` label so multi-device consumers
    // attribute them correctly.
    /// A flow file could not be read, parsed, or mixin-expanded.
    FlowParseFailed {
        path: String,
        error: String,
    },
    /// No booted device matched a slot; scheduler is booting a shutdown
    /// one to satisfy it.
    DeviceAutoBoot {
        device_name: String,
        /// Pre-formatted slot shape, e.g. `ios/v26/phone`.
        slot_shape: String,
    },
    /// Auto-boot completed successfully. Lets streams render boot duration
    /// alongside install / companion timings.
    DeviceAutoBootFinished {
        device_name: String,
        slot_shape: String,
        duration_ms: u64,
    },
    /// A slot couldn't acquire a device, companion, or allocation and was
    /// skipped. The worker still emits a failed FlowReport; this event
    /// surfaces the reason to live consumers.
    SlotSetupFailed {
        /// Pre-formatted slot descriptor including apps.
        slot_label: String,
        reason: String,
    },
    /// Allocation backoff: RAM + max-concurrency caps are full, waiting
    /// for another device to release.
    ResourcesWaiting {
        platform: String,
    },
    /// Companion binary has been launched and we're waiting for it to
    /// register + health-check.
    CompanionStarting {
        platform: String,
        device_name: String,
    },
    /// Companion finished health check and is ready to accept driver
    /// requests.
    CompanionReady {
        platform: String,
        version: String,
        device_name: String,
        os_version: String,
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

    // Post-action UI settle, emitted out-of-band (not inside any
    // step's timeout). Surfaces the wall-clock between an action
    // completing and the next step starting, so a slow settle is
    // visible when triaging intermittent failures on long sweeps.
    PostSettle {
        action: String,
        duration_ms: u64,
        stable: bool,
    },

    // App lifecycle
    AppLaunch {
        bundle: String,
        duration_ms: u64,
    },
    AppStop {
        bundle: String,
    },
    /// Driver-level warning surfaced inside a step. Currently emitted
    /// when the iOS companion's launch settle-probe times out — the
    /// app is foregrounded but the WebView's first paint may not be
    /// ready, so the next interaction can race. Step still passes;
    /// this is a breadcrumb for downstream-failure diagnosis.
    DriverWarning {
        message: String,
    },

    // Media
    Screenshot {
        path: String,
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
