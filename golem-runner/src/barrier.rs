//! Failure barrier for cross-device step synchronization.
//!
//! When multiple devices run the same flow in parallel, a failure barrier
//! lets them coordinate: if device A fails at step N, device B continues
//! until it completes step N, then stops. This reveals whether a failure
//! is platform-specific or universal.

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
}
