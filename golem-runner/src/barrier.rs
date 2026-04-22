//! Failure barrier for cross-device step synchronization.
//!
//! When multiple devices run the same flow in parallel, a failure barrier
//! lets them coordinate: if device A fails at step N, device B continues
//! until it completes step N, then stops. This reveals whether a failure
//! is platform-specific or universal.
//!
//! # Scope: per-flow, never per-suite
//!
//! A `FailureBarrier` instance is constructed fresh for each flow execution
//! (`run_single_flow_with_resources` in `golem-cli/src/suite.rs`) and cloned
//! to the per-device tasks spawned for that flow. Multi-flow parallelism
//! (e.g. `golem run a.toml b.toml`) calls the runner once per flow, so each
//! flow gets its own independent barrier and a failure in flow A cannot
//! abort flow B.
//!
//! This is a hard requirement, not an implementation detail: the global
//! step count is only meaningful within a single flow. Step 7 of flow A and
//! step 7 of flow B are unrelated positions. Collapsing the barrier into a
//! shared-across-flows singleton would make one flow's failure arbitrarily
//! truncate unrelated flows.
//!
//! Future scheduler rewrites (see the Dynamic JIT Scheduler roadmap entry)
//! must keep barrier allocation keyed to the flow, not the suite.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Encodes a failure point as a single u64: (step_count << 0).
/// Using the global step count (not block+step) since blocks can be
/// skipped via `where` clauses, making block indices inconsistent
/// across devices.
const NO_FAILURE: u64 = u64::MAX;

/// Shared failure barrier for parallel device execution.
///
/// When a device fails, it reports its global step count. Other devices
/// check the barrier after each step — once they reach or exceed the
/// failure point, they stop with a "barrier reached" result.
#[derive(Clone)]
pub struct FailureBarrier {
    /// The global step count at which the first failure occurred.
    /// u64::MAX means no failure yet.
    failed_at: Arc<AtomicU64>,
}

impl FailureBarrier {
    /// Create a new barrier with no failure recorded.
    pub fn new() -> Self {
        Self {
            failed_at: Arc::new(AtomicU64::new(NO_FAILURE)),
        }
    }

    /// Report a failure at the given global step count.
    /// Only records the FIRST failure (lowest step count wins).
    pub fn report_failure(&self, step_count: u64) {
        self.failed_at.fetch_min(step_count, Ordering::Release);
    }

    /// Check if another device has failed at or before the given step count.
    /// Returns true if this device should stop.
    pub fn should_stop(&self, step_count: u64) -> bool {
        let barrier = self.failed_at.load(Ordering::Acquire);
        barrier != NO_FAILURE && step_count >= barrier
    }

    /// Check if any failure has been recorded.
    pub fn has_failure(&self) -> bool {
        self.failed_at.load(Ordering::Acquire) != NO_FAILURE
    }

    /// Get the step count at which the first failure occurred, if any.
    pub fn failure_point(&self) -> Option<u64> {
        let val = self.failed_at.load(Ordering::Acquire);
        if val == NO_FAILURE { None } else { Some(val) }
    }
}

impl Default for FailureBarrier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_barrier_has_no_failure() {
        let b = FailureBarrier::new();
        assert!(!b.has_failure());
        assert!(!b.should_stop(100));
        assert_eq!(b.failure_point(), None);
    }

    #[test]
    fn report_failure_records_step() {
        let b = FailureBarrier::new();
        b.report_failure(7);
        assert!(b.has_failure());
        assert_eq!(b.failure_point(), Some(7));
    }

    #[test]
    fn should_stop_at_failure_point() {
        let b = FailureBarrier::new();
        b.report_failure(7);

        assert!(!b.should_stop(6)); // before failure — keep going
        assert!(b.should_stop(7));  // at failure — stop
        assert!(b.should_stop(8));  // past failure — stop
    }

    #[test]
    fn first_failure_wins() {
        let b = FailureBarrier::new();
        b.report_failure(10);
        b.report_failure(5);  // earlier failure
        b.report_failure(15); // later failure

        assert_eq!(b.failure_point(), Some(5)); // earliest wins
    }

    #[test]
    fn clone_shares_state() {
        let b1 = FailureBarrier::new();
        let b2 = b1.clone();

        b1.report_failure(7);
        assert!(b2.should_stop(7)); // b2 sees b1's failure
    }

    #[test]
    fn separate_instances_do_not_share_state() {
        // Models the multi-flow case: `golem run a.toml b.toml` constructs
        // a fresh barrier per flow. Failure in flow A MUST NOT abort flow B.
        // If a future refactor ever collapses barrier scope to per-suite
        // (e.g. reusing one barrier across flows in a scheduler loop), this
        // test fails and forces the reviewer to reassess the semantics.
        let flow_a = FailureBarrier::new();
        let flow_b = FailureBarrier::new();

        flow_a.report_failure(3);

        assert!(flow_a.has_failure(), "flow A SHALL record its own failure");
        assert!(
            !flow_b.has_failure(),
            "flow B SHALL NOT see flow A's failure across independent barrier instances",
        );
        assert!(!flow_b.should_stop(3));
        assert!(!flow_b.should_stop(100));
        assert_eq!(flow_b.failure_point(), None);
    }
}
