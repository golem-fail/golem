// golem-events: structured event stream for test execution
#![deny(clippy::unwrap_used)]

pub mod channel;
pub mod code;
pub mod emitter;

pub use code::{clean_msg, coded, extract_code, CodeExt, CodedError, Domain, FailureCode, Severity};

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
    Warning { message: String, code: FailureCode },
    Failed { message: String, code: FailureCode },
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
        /// Flow-level failure code (e.g. max_runtime EF504 / max_steps EF508).
        /// `None` on success, or when a failing step already carries the code
        /// (the stream surfaces that). Lets the human FAIL line show a code for
        /// flow-level aborts that have no owning step.
        code: Option<FailureCode>,
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
        /// Failure code when failed (A-domain).
        code: Option<FailureCode>,
        /// Pre-formatted target (same as InstallStarted.target).
        target: String,
        /// OS major version (same as InstallStarted.os_major).
        os_major: u32,
    },
    /// A flow did not run, and that is *not* a failure — a deliberate skip
    /// or an informational notice. Examples: a peer run satisfied a coverage
    /// group, or a device is being rebooted in the background for ANR
    /// recovery (the triggering flow's own failure is reported separately).
    /// Counts as success (exit 0).
    FlowSkipped {
        flow_name: String,
        reason: String,
    },
    /// A flow could not run because a precondition failed (missing bundle id,
    /// failed or absent install script). The flow never executed a step, so
    /// it counts as a failure (exit 1) and carries the responsible code.
    FlowCouldNotRun {
        flow_name: String,
        reason: String,
        code: FailureCode,
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

#[cfg(test)]
mod tests {
    use super::*;

    // 1. DeviceId Display writes the inner string verbatim.
    #[test]
    fn device_id_display_is_verbatim() {
        assert_eq!(DeviceId("ios/iPhone 15 Pro".into()).to_string(), "ios/iPhone 15 Pro");
        assert_eq!(DeviceId("suite".into()).to_string(), "suite");
        assert_eq!(DeviceId(String::new()).to_string(), "");
    }

    // 2. system_time_to_unix_nanos on a known epoch offset returns that
    //    offset in nanoseconds.
    #[test]
    fn system_time_to_unix_nanos_known_offset() {
        let t = UNIX_EPOCH + std::time::Duration::new(5, 123);
        assert_eq!(
            system_time_to_unix_nanos(t),
            5_000_000_123,
            "secs+subsec SHALL combine into total nanos"
        );
    }

    // 3. Pre-epoch SystemTime saturates to 0 rather than panicking.
    #[test]
    fn system_time_to_unix_nanos_pre_epoch_saturates_zero() {
        let pre = UNIX_EPOCH - std::time::Duration::from_secs(10);
        assert_eq!(
            system_time_to_unix_nanos(pre),
            0,
            "pre-epoch time SHALL saturate to 0"
        );
    }

    // 4. unix_nanos_to_system_time reconstructs the same SystemTime
    //    (round-trips with the forward conversion).
    #[test]
    fn unix_nanos_roundtrip_through_system_time() {
        let nanos = 1_700_000_000_987_654_321u128;
        let t = unix_nanos_to_system_time(nanos);
        assert_eq!(
            system_time_to_unix_nanos(t),
            nanos,
            "nanos SHALL round-trip through SystemTime"
        );
    }

    // 5. unix_nanos_to_system_time splits secs/subsec without u64
    //    truncation for a post-2554 value (> u64::MAX nanos).
    #[test]
    fn unix_nanos_to_system_time_handles_post_2554() {
        // A value larger than u64::MAX nanoseconds would truncate if the
        // split were done with Duration::from_nanos on the raw u128.
        let big = (u64::MAX as u128) + 2_000_000_000; // ~2 secs past u64 ns
        let t = unix_nanos_to_system_time(big);
        assert_eq!(
            system_time_to_unix_nanos(t),
            big,
            "post-2554 nanos SHALL survive without truncation"
        );
    }

    // 6. unix_nanos_to_system_time at exactly the epoch yields UNIX_EPOCH.
    #[test]
    fn unix_nanos_zero_is_epoch() {
        assert_eq!(unix_nanos_to_system_time(0), UNIX_EPOCH);
    }

    fn sample_event() -> Event {
        Event {
            seq: 42,
            device_id: DeviceId("android/Pixel_7_API_34".into()),
            timestamp: Instant::now(),
            wall_time: UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 250),
            kind: EventKind::SuiteStarted { flow_count: 3 },
        }
    }

    // 7. WireEvent::from(&Event) copies seq/device_id/kind and encodes
    //    wall_time as unix nanos.
    #[test]
    fn wire_event_from_event_copies_fields() {
        let e = sample_event();
        let w = WireEvent::from(&e);
        assert_eq!(w.seq, 42);
        assert_eq!(w.device_id, DeviceId("android/Pixel_7_API_34".into()));
        assert_eq!(
            w.wall_time_unix_nanos,
            system_time_to_unix_nanos(e.wall_time),
            "wire wall_time SHALL be the unix-nanos encoding"
        );
        assert!(matches!(w.kind, EventKind::SuiteStarted { flow_count: 3 }));
    }

    // 8. WireEvent::into_event reconstructs wall_time from nanos and
    //    preserves seq/device_id/kind; timestamp is a fresh Instant.
    #[test]
    fn wire_event_into_event_reconstructs_wall_time() {
        let e = sample_event();
        let original_wall = e.wall_time;
        let w = WireEvent::from(&e);
        let back = w.into_event();
        assert_eq!(back.seq, 42);
        assert_eq!(back.device_id, DeviceId("android/Pixel_7_API_34".into()));
        assert_eq!(
            back.wall_time, original_wall,
            "rehydrated wall_time SHALL match the original"
        );
        assert!(matches!(back.kind, EventKind::SuiteStarted { flow_count: 3 }));
    }

    // 9. WireEvent serializes to the exact wire schema: a flat object keyed
    //    `seq`/`device_id`/`wall_time_unix_nanos`/`kind`, with device_id as a
    //    bare string (newtype-transparent) and wall_time as a JSON number.
    //    Guards against schema drift (renamed keys, DeviceId gaining a wrapper
    //    object, wall_time becoming a quoted string), which a symmetric
    //    encode/decode would silently pass.
    #[test]
    fn wire_event_serde_roundtrip() {
        let w = WireEvent::from(&sample_event());
        let json = serde_json::to_string(&w).expect("serialize SHALL succeed");
        // wall_time of sample_event() is epoch + 1_700_000_000s + 250ns.
        assert_eq!(
            json,
            r#"{"seq":42,"device_id":"android/Pixel_7_API_34","wall_time_unix_nanos":1700000000000000250,"kind":{"SuiteStarted":{"flow_count":3}}}"#,
            "WireEvent JSON SHALL be the flat wire schema with a bare-string device_id and numeric wall_time"
        );
        let back: WireEvent = serde_json::from_str(&json).expect("deserialize SHALL succeed");
        assert_eq!(back.seq, 42);
        assert_eq!(back.device_id, DeviceId("android/Pixel_7_API_34".into()));
        assert_eq!(back.wall_time_unix_nanos, 1_700_000_000_000_000_250);
    }

    // 10. TreeStats::record on the first fetch seeds both min and max.
    #[test]
    fn tree_stats_record_first_fetch_seeds_min_max() {
        let mut s = TreeStats::default();
        s.record(57);
        assert_eq!(s.fetches, 1);
        assert_eq!(s.min_nodes, 57, "first fetch SHALL seed min_nodes");
        assert_eq!(s.max_nodes, 57, "first fetch SHALL seed max_nodes");
    }

    // 11. TreeStats::record tracks min and max across fetches, including a
    //     later smaller count than the first.
    #[test]
    fn tree_stats_record_tracks_min_and_max() {
        let mut s = TreeStats::default();
        s.record(50);
        s.record(80);
        s.record(30);
        assert_eq!(s.fetches, 3);
        assert_eq!(s.min_nodes, 30, "min SHALL fall to the smallest count");
        assert_eq!(s.max_nodes, 80, "max SHALL rise to the largest count");
    }

    // 12. TreeStats::record can seed min to 0 on the first fetch (0 is a
    //     legitimate first value, distinct from the merge sentinel).
    #[test]
    fn tree_stats_record_zero_first_fetch() {
        let mut s = TreeStats::default();
        s.record(0);
        s.record(10);
        assert_eq!(s.min_nodes, 0, "0 first count SHALL stay as min");
        assert_eq!(s.max_nodes, 10);
    }

    // 13. TreeStats::merge with a zero-fetch other is a no-op.
    #[test]
    fn tree_stats_merge_zero_fetch_is_noop() {
        let mut s = TreeStats { fetches: 2, min_nodes: 10, max_nodes: 40 };
        s.merge(&TreeStats::default());
        assert_eq!(s.fetches, 2);
        assert_eq!(s.min_nodes, 10);
        assert_eq!(s.max_nodes, 40);
    }

    // 14. TreeStats::merge into a fresh (min_nodes==0 sentinel) target adopts
    //     the other's min rather than keeping the 0 sentinel.
    #[test]
    fn tree_stats_merge_into_fresh_adopts_min() {
        let mut s = TreeStats::default();
        let other = TreeStats { fetches: 3, min_nodes: 12, max_nodes: 90 };
        s.merge(&other);
        assert_eq!(s.fetches, 3);
        assert_eq!(s.min_nodes, 12, "fresh target SHALL take other's min, not 0");
        assert_eq!(s.max_nodes, 90);
    }

    // 15. TreeStats::merge combines fetch counts and takes the lower min /
    //     higher max across both.
    #[test]
    fn tree_stats_merge_combines_min_max() {
        let mut s = TreeStats { fetches: 2, min_nodes: 20, max_nodes: 50 };
        let other = TreeStats { fetches: 4, min_nodes: 15, max_nodes: 45 };
        s.merge(&other);
        assert_eq!(s.fetches, 6, "fetch counts SHALL add");
        assert_eq!(s.min_nodes, 15, "merge SHALL keep the lower min");
        assert_eq!(s.max_nodes, 50, "merge SHALL keep the higher max");
    }

    // 16. TreeStats::merge does not lower a nonzero min when other's min is
    //     larger.
    #[test]
    fn tree_stats_merge_keeps_smaller_existing_min() {
        let mut s = TreeStats { fetches: 1, min_nodes: 5, max_nodes: 5 };
        let other = TreeStats { fetches: 1, min_nodes: 100, max_nodes: 200 };
        s.merge(&other);
        assert_eq!(s.min_nodes, 5, "existing smaller min SHALL be preserved");
        assert_eq!(s.max_nodes, 200);
    }

    // 17. EventKind StepFinished serializes to the externally-tagged wire
    //     schema: `{"StepFinished":{...}}`, with the nested StepOutcome also
    //     externally tagged (`{"Failed":{...}}`) and FailureCode as its bare
    //     variant name (`"FlowElementNotFound"`). Asserting the literal shape
    //     catches an unintended enum-representation change (e.g. someone
    //     adding `#[serde(tag=...)]` or renaming a code) that a symmetric
    //     round-trip would not detect.
    #[test]
    fn step_finished_serde_roundtrip() {
        let kind = EventKind::StepFinished {
            global_step_index: 7,
            outcome: StepOutcome::Failed {
                message: "boom".into(),
                code: FailureCode::FlowElementNotFound,
            },
            duration_ms: 1234,
            retry_count: 2,
            screenshot_path: Some("/tmp/shot.png".into()),
            tree_stats: TreeStats { fetches: 3, min_nodes: 10, max_nodes: 20 },
        };
        let json = serde_json::to_string(&kind).expect("serialize SHALL succeed");
        assert_eq!(
            json,
            r#"{"StepFinished":{"global_step_index":7,"outcome":{"Failed":{"message":"boom","code":"FlowElementNotFound"}},"duration_ms":1234,"retry_count":2,"screenshot_path":"/tmp/shot.png","tree_stats":{"fetches":3,"min_nodes":10,"max_nodes":20}}}"#,
            "EventKind SHALL be externally tagged with FailureCode as a bare variant name"
        );
        let back: EventKind = serde_json::from_str(&json).expect("deserialize SHALL succeed");
        match back {
            EventKind::StepFinished { global_step_index, retry_count, outcome, .. } => {
                assert_eq!(global_step_index, 7);
                assert_eq!(retry_count, 2);
                assert!(matches!(
                    outcome,
                    StepOutcome::Failed { code: FailureCode::FlowElementNotFound, .. }
                ));
            }
            other => panic!("expected StepFinished, got {other:?}"),
        }
    }

    // 19. SubstepEvent ScrollAttempt serializes to the externally-tagged wire
    //     schema: `{"ScrollAttempt":{...}}`, with nested Point objects keyed
    //     `x`/`y` and ScrollAttemptResult externally tagged
    //     (`{"Stall":{"count":..,"max":..}}`). The literal guards the nested
    //     geometry + result enum representation against drift that a symmetric
    //     encode/decode would silently pass.
    #[test]
    fn substep_scroll_attempt_serde_roundtrip() {
        let s = SubstepEvent::ScrollAttempt {
            attempt: 1,
            direction: "down".into(),
            strategy_index: 0,
            from: Point { x: 10, y: 200 },
            to: Point { x: 10, y: 50 },
            result: ScrollAttemptResult::Stall { count: 2, max: 3 },
            tree_stats: TreeStats::default(),
        };
        let json = serde_json::to_string(&s).expect("serialize SHALL succeed");
        assert_eq!(
            json,
            r#"{"ScrollAttempt":{"attempt":1,"direction":"down","strategy_index":0,"from":{"x":10,"y":200},"to":{"x":10,"y":50},"result":{"Stall":{"count":2,"max":3}},"tree_stats":{"fetches":0,"min_nodes":0,"max_nodes":0}}}"#,
            "SubstepEvent SHALL be externally tagged with x/y Points and a tagged Stall result"
        );
        let back: SubstepEvent = serde_json::from_str(&json).expect("deserialize SHALL succeed");
        match back {
            SubstepEvent::ScrollAttempt { from, to, result, .. } => {
                assert_eq!(from.x, 10);
                assert_eq!(to.y, 50);
                assert!(matches!(result, ScrollAttemptResult::Stall { count: 2, max: 3 }));
            }
            other => panic!("expected ScrollAttempt, got {other:?}"),
        }
    }
}
