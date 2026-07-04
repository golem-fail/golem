//! Host-wide serialization queue for device ops that contend on a single
//! shared host resource.
//!
//! When golem drives several devices in parallel, most ops are per-device and
//! safe to run concurrently (a tap on device A can't disturb device B). A few
//! ops route through a resource that is shared across *all* devices on the
//! host, and running those concurrently either thrashes the resource (I/O
//! bursts) or corrupts shared state:
//!
//! - **Android** — every `adb` invocation multiplexes through the one host
//!   `adb server`. Big transfers ([`OpClass::AdbHostIo`], e.g. `adb pull` of a
//!   screenrecord) and framebuffer captures ([`OpClass::Screenshot`]) saturate
//!   it under a concurrent burst; `dumpsys` walks ([`OpClass::Dumpsys`]) pile
//!   on. Small `adb shell` commands stay in the parallel lane.
//! - **iOS** — every `xcrun simctl` call funnels through the single host
//!   `CoreSimulatorService` daemon ([`OpClass::Simctl`]). Per-sim companion
//!   ops (tap/type/hierarchy/screenshot via XCUITest) run on each sim's own
//!   main thread and are left parallel.
//!
//! Each [`OpClass`] gets its own process-global `Semaphore(1)`, so ops of the
//! same class serialize host-wide while different classes (and unclassified
//! ops) still run concurrently. Because a single test flow is sequential per
//! device, the queue for any class holds at most one entry per device — there
//! is no need to reorder or preempt.
//!
//! ## Leaf granularity
//!
//! Wrap the *leaf* op — one `adb` invocation, one companion roundtrip, one
//! `simctl` call — never a driver method that also performs golem-side settle
//! polling. Holding a permit across a settle loop would block every other
//! device for the poll duration; wrapping the leaf keeps each hold to a single
//! short roundtrip so the queue drains fast.
//!
//! ## Test isolation
//!
//! The registry is process-global. The concurrency tests below assert on
//! host-wide serialization, so they assume one test per process — the
//! workspace runs under `cargo nextest` (process-per-test). They are not
//! reliable under multi-threaded `cargo test`, where a sibling test sharing an
//! `OpClass` would perturb the observed overlap.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::time::Instant;

/// A class of host-contended op. Ops of the same class serialize host-wide;
/// different classes run concurrently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpClass {
    /// Bulk `adb` host I/O (e.g. `adb pull` of a screenrecord file) — shares
    /// the one host `adb server` transport.
    AdbHostIo,
    /// Framebuffer capture (`/screenshot`) — heavy bytes over the shared adb
    /// transport.
    Screenshot,
    /// `adb shell dumpsys …` walks.
    Dumpsys,
    /// Short, per-command `xcrun simctl …` action ops (location, openurl,
    /// appearance, terminate, privacy, …) — the single host
    /// `CoreSimulatorService` daemon.
    ///
    /// INVARIANT: only sub-second action ops belong here. Long lifecycle
    /// `simctl` calls (`boot`/reboot, multi-second) deliberately stay OFF this
    /// class — they run on the golem-common command-seam path in device
    /// bring-up / reboot recovery, not the driver's `simctl()` funnel. Keeping
    /// boot out means a device being (re)booted can never hold this permit
    /// long enough to false-timeout another device's step-bounded action. Do
    /// not route `boot` through this class without adding a separate long-op
    /// class first.
    Simctl,
}

impl OpClass {
    /// Stable index into the per-class stats arrays.
    fn idx(self) -> usize {
        match self {
            OpClass::AdbHostIo => 0,
            OpClass::Screenshot => 1,
            OpClass::Dumpsys => 2,
            OpClass::Simctl => 3,
        }
    }

    /// Short label for congestion reporting.
    pub fn label(self) -> &'static str {
        match self {
            OpClass::AdbHostIo => "adb-io",
            OpClass::Screenshot => "screenshot",
            OpClass::Dumpsys => "dumpsys",
            OpClass::Simctl => "simctl",
        }
    }

    /// All classes, in `idx` order.
    const ALL: [OpClass; N_CLASSES] = [
        OpClass::AdbHostIo,
        OpClass::Screenshot,
        OpClass::Dumpsys,
        OpClass::Simctl,
    ];
}

const N_CLASSES: usize = 4;

// Cumulative permit-wait per class. `WAIT_MICROS` sums every acquire's wait
// (≈0 when uncontended); `WAIT_COUNT` counts only acquires that actually
// blocked (>1ms), so the reported count reflects real contention, not the
// microsecond noise of an instant uncontended acquire.
#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static WAIT_MICROS: [AtomicU64; N_CLASSES] = [ZERO; N_CLASSES];
static WAIT_COUNT: [AtomicU64; N_CLASSES] = [ZERO; N_CLASSES];

/// Per-class permit-wait accrued since the last [`reset_queue_wait_stats`].
#[derive(Debug, Clone)]
pub struct ClassWait {
    pub class: OpClass,
    pub waited: Duration,
    pub count: u64,
}

/// Snapshot of host-queue congestion: how long ops spent waiting for a permit,
/// broken down by class. Zero when no two same-class ops ever overlapped
/// (single device, or cross-platform ops that never share a class).
#[derive(Debug, Clone)]
pub struct QueueWaitStats {
    /// Classes that recorded a blocking wait, most-waited first.
    pub per_class: Vec<ClassWait>,
    pub total: Duration,
}

impl QueueWaitStats {
    pub fn is_zero(&self) -> bool {
        self.total.is_zero()
    }
}

/// Snapshot the accumulated per-class permit-wait.
pub fn queue_wait_stats() -> QueueWaitStats {
    let mut per_class = Vec::new();
    let mut total = Duration::ZERO;
    for class in OpClass::ALL {
        let micros = WAIT_MICROS[class.idx()].load(Relaxed);
        total += Duration::from_micros(micros);
        let count = WAIT_COUNT[class.idx()].load(Relaxed);
        if count > 0 {
            per_class.push(ClassWait {
                class,
                waited: Duration::from_micros(micros),
                count,
            });
        }
    }
    per_class.sort_by(|a, b| b.waited.cmp(&a.waited));
    QueueWaitStats { per_class, total }
}

/// Zero the congestion counters. Call once at the start of a run so the
/// summary reflects only that run (statics are process-global and a reused
/// daemon would otherwise accumulate across invocations).
pub fn reset_queue_wait_stats() {
    for i in 0..N_CLASSES {
        WAIT_MICROS[i].store(0, Relaxed);
        WAIT_COUNT[i].store(0, Relaxed);
    }
}

/// Per-class `Semaphore(1)` registry, created lazily on first use.
fn registry() -> &'static Mutex<HashMap<OpClass, Arc<Semaphore>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<OpClass, Arc<Semaphore>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The (shared, single-permit) semaphore for `class`.
fn class_semaphore(class: OpClass) -> Arc<Semaphore> {
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(
        map.entry(class)
            .or_insert_with(|| Arc::new(Semaphore::new(1))),
    )
}

/// Run `fut` while holding `class`'s host-wide permit, releasing it as soon as
/// `fut` completes (or unwinds — the permit is RAII-scoped).
///
/// The permit is acquired *before* `fut` is polled, so callers that also apply
/// a per-op timeout should wrap only `fut`, not this call, to keep permit-wait
/// out of the timed budget.
pub async fn acquire_then_run<F, T>(class: OpClass, fut: F) -> T
where
    F: Future<Output = T>,
{
    let sem = class_semaphore(class);
    // Time the acquire: ≈0 when the permit is free, = the holder's remaining
    // hold time when contended. Uses tokio's clock so it reflects real time in
    // production and virtual time under `start_paused` tests.
    let wait_start = Instant::now();
    // The registry never closes its semaphores, so acquire cannot fail.
    let permit = sem
        .acquire_owned()
        .await
        .expect("host_queue semaphore is never closed");
    let waited = wait_start.elapsed();
    let idx = class.idx();
    WAIT_MICROS[idx].fetch_add(waited.as_micros() as u64, Relaxed);
    if waited >= Duration::from_millis(1) {
        WAIT_COUNT[idx].fetch_add(1, Relaxed);
    }
    // Tripwire: a single acquire this slow means heavy host-wide contention.
    // It's still well under a typical step timeout at the loads we've measured,
    // but if it ever climbs toward one, this greppable line is the early signal
    // that permit-wait — not slow work — is the risk (see the deliberately
    // unbuilt step-timeout exclusion). Always emitted; slow waits are rare.
    if should_warn_slow_wait(waited) {
        eprintln!(
            "  [host-queue] slow permit wait: {} blocked {:.1}s behind other \
             devices (host-wide serialization under heavy load)",
            class.label(),
            waited.as_secs_f64()
        );
    }
    let result = fut.await;
    drop(permit);
    result
}

/// A single acquire waiting at least this long trips the slow-wait warning.
const SLOW_WAIT_WARN: Duration = Duration::from_secs(2);

fn should_warn_slow_wait(waited: Duration) -> bool {
    waited >= SLOW_WAIT_WARN
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    /// Enter a critical section, record peak concurrency, yield enough times
    /// that a *concurrent* entrant would be observed, then leave. If the class
    /// semaphore serializes, `peak` never exceeds 1.
    async fn record_overlap(cur: &AtomicUsize, peak: &AtomicUsize) {
        let now = cur.fetch_add(1, SeqCst) + 1;
        peak.fetch_max(now, SeqCst);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        cur.fetch_sub(1, SeqCst);
    }

    // 1. Two ops of the SAME class never overlap — the second waits for the
    //    first to release, so peak concurrency is 1.
    #[tokio::test]
    async fn same_class_serializes() {
        let cur = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        tokio::join!(
            acquire_then_run(OpClass::Screenshot, record_overlap(&cur, &peak)),
            acquire_then_run(OpClass::Screenshot, record_overlap(&cur, &peak)),
        );
        assert_eq!(
            peak.load(SeqCst),
            1,
            "same-class ops SHALL NOT run concurrently"
        );
        assert_eq!(cur.load(SeqCst), 0, "all permits SHALL be released");
    }

    // 2. Ops of DIFFERENT classes run concurrently — each holds its own
    //    class permit, so both are in the section at once (peak 2).
    #[tokio::test]
    async fn different_classes_run_concurrently() {
        let cur = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        tokio::join!(
            acquire_then_run(OpClass::Dumpsys, record_overlap(&cur, &peak)),
            acquire_then_run(OpClass::Simctl, record_overlap(&cur, &peak)),
        );
        assert_eq!(
            peak.load(SeqCst),
            2,
            "distinct-class ops SHALL run concurrently"
        );
    }

    // 3. The permit is released on completion: a second same-class op after
    //    the first returns proceeds without deadlock, and the closure's value
    //    passes through unchanged.
    #[tokio::test]
    async fn permit_released_after_completion() {
        let first = acquire_then_run(OpClass::AdbHostIo, async { 1u32 }).await;
        let second = acquire_then_run(OpClass::AdbHostIo, async { 2u32 }).await;
        assert_eq!((first, second), (1, 2), "sequential reuse SHALL not deadlock");
    }

    // 4. A permit taken by an op that returns an `Err` is still released — the
    //    RAII guard drops regardless of the future's output — so a following
    //    same-class op runs.
    #[tokio::test]
    async fn permit_released_after_err_output() {
        let e: Result<(), &str> =
            acquire_then_run(OpClass::Dumpsys, async { Err("boom") }).await;
        assert!(e.is_err(), "error output SHALL pass through");
        let ok: Result<u8, &str> = acquire_then_run(OpClass::Dumpsys, async { Ok(7) }).await;
        assert_eq!(ok, Ok(7), "permit SHALL be free after an Err-returning op");
    }

    // 5. An uncontended op records no meaningful wait — a lone acquire on a
    //    free permit is instant, so the count stays 0 and total is ~0.
    #[tokio::test]
    async fn uncontended_records_no_wait() {
        reset_queue_wait_stats();
        acquire_then_run(OpClass::AdbHostIo, async {}).await;
        let stats = queue_wait_stats();
        assert!(
            stats.per_class.is_empty(),
            "an uncontended acquire SHALL record no blocking wait, got {:?}",
            stats.per_class
        );
        assert!(
            stats.total < Duration::from_millis(1),
            "uncontended total SHALL be ~0, got {:?}",
            stats.total
        );
    }

    // 6. When a second same-class op waits behind a permit held for a known
    //    duration, that wait is recorded against the class (virtual time via
    //    start_paused makes the ~held duration deterministic).
    #[tokio::test(start_paused = true)]
    async fn contended_wait_is_recorded_against_class() {
        reset_queue_wait_stats();
        // Holder keeps the Screenshot permit for 1s; the waiter blocks on it.
        let holder = acquire_then_run(OpClass::Screenshot, async {
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let waiter = acquire_then_run(OpClass::Screenshot, async {});
        tokio::join!(holder, waiter);

        let stats = queue_wait_stats();
        assert_eq!(stats.per_class.len(), 1, "only Screenshot SHALL show a wait");
        let sw = &stats.per_class[0];
        assert_eq!(sw.class, OpClass::Screenshot);
        assert_eq!(sw.count, 1, "exactly one acquire SHALL have blocked");
        assert!(
            sw.waited >= Duration::from_millis(900),
            "recorded wait SHALL be ~the 1s hold, got {:?}",
            sw.waited
        );
    }

    // 7. reset_queue_wait_stats drains the counters so a later snapshot starts
    //    fresh.
    #[tokio::test(start_paused = true)]
    async fn reset_drains_wait_stats() {
        let holder = acquire_then_run(OpClass::Dumpsys, async {
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let waiter = acquire_then_run(OpClass::Dumpsys, async {});
        tokio::join!(holder, waiter);
        assert!(!queue_wait_stats().is_zero(), "wait SHALL be recorded");

        reset_queue_wait_stats();
        let stats = queue_wait_stats();
        assert!(stats.is_zero(), "reset SHALL zero the counters");
        assert!(stats.per_class.is_empty(), "reset SHALL clear per-class rows");
    }

    // 8. The slow-wait tripwire fires only at/above the 2s threshold — brief
    //    contention (the sub-second waits we measured at 2-4 emus) stays quiet.
    #[test]
    fn slow_wait_tripwire_threshold() {
        assert!(
            !should_warn_slow_wait(Duration::from_millis(94)),
            "typical 4-emu wait SHALL NOT trip the warning"
        );
        assert!(
            !should_warn_slow_wait(Duration::from_millis(1999)),
            "just under threshold SHALL NOT warn"
        );
        assert!(
            should_warn_slow_wait(SLOW_WAIT_WARN),
            "a wait at the threshold SHALL warn"
        );
        assert!(
            should_warn_slow_wait(Duration::from_secs(5)),
            "a long wait SHALL warn"
        );
    }
}
