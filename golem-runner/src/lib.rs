// golem-runner: test execution orchestrator

use std::sync::atomic::{AtomicU32, Ordering};

/// Return early from a function with a [`golem_events::FailureCode`]-tagged
/// `anyhow` error. Drop-in for `anyhow::bail!` at sites whose cause maps to a
/// registry code — the tag rides the error chain to the capture site, where
/// `extract_code` surfaces it in output.
#[macro_export]
macro_rules! fail_code {
    ($code:expr, $($arg:tt)*) => {
        return ::core::result::Result::Err(
            ::golem_events::coded($code, ::anyhow::anyhow!($($arg)*))
        )
    };
}

/// Per-step tree fetch counter. Reset by executor before each step,
/// incremented by wait_for_settle on each get_hierarchy call.
static TREE_FETCH_COUNT: AtomicU32 = AtomicU32::new(0);
static TREE_NODE_MIN: AtomicU32 = AtomicU32::new(0);
static TREE_NODE_MAX: AtomicU32 = AtomicU32::new(0);

pub fn reset_step_tree_stats() {
    TREE_FETCH_COUNT.store(0, Ordering::Relaxed);
    TREE_NODE_MIN.store(u32::MAX, Ordering::Relaxed);
    TREE_NODE_MAX.store(0, Ordering::Relaxed);
}

pub fn record_tree_fetch(node_count: u32) {
    TREE_FETCH_COUNT.fetch_add(1, Ordering::Relaxed);
    TREE_NODE_MIN.fetch_min(node_count, Ordering::Relaxed);
    TREE_NODE_MAX.fetch_max(node_count, Ordering::Relaxed);
}

pub fn take_step_tree_stats() -> golem_events::TreeStats {
    let fetches = TREE_FETCH_COUNT.swap(0, Ordering::Relaxed);
    let min = TREE_NODE_MIN.swap(u32::MAX, Ordering::Relaxed);
    let max = TREE_NODE_MAX.swap(0, Ordering::Relaxed);
    golem_events::TreeStats {
        fetches,
        min_nodes: if fetches == 0 { 0 } else { min },
        max_nodes: max,
    }
}

pub mod actions;
pub mod barrier;
pub mod capture;
pub mod cleanup;
pub mod context;
pub mod branch;
pub mod fixture_loader;
pub mod data_driven;
pub mod device_vars;
pub mod executor;
pub mod fingerprint;
pub mod installed_state;
pub mod installer;
pub mod loops;
pub mod policy;
pub mod resolution;
pub mod scroll;
pub mod subflow;
pub mod parallel;
pub mod for_each;
pub mod perf;
pub mod teardown;

#[cfg(test)]
mod tests {
    use super::*;

    // Helper using `fail_code!`, which expands to an early `return Err(coded(..))`.
    fn fails_with(code: golem_events::FailureCode) -> anyhow::Result<()> {
        fail_code!(code, "boom {}", 42);
    }

    // 1. After reset, taking stats with no fetches yields zeroed stats and
    //    min_nodes is clamped to 0 (not the sentinel u32::MAX) when fetches==0.
    #[test]
    fn take_after_reset_with_no_fetches_is_all_zero() {
        reset_step_tree_stats();
        let stats = take_step_tree_stats();
        assert_eq!(stats.fetches, 0, "no fetches SHALL report zero fetches");
        assert_eq!(stats.min_nodes, 0, "zero fetches SHALL clamp min_nodes to 0");
        assert_eq!(stats.max_nodes, 0, "zero fetches SHALL report zero max_nodes");
    }

    // 2. A single recorded fetch reports min==max==that node count and one fetch.
    #[test]
    fn single_fetch_min_equals_max() {
        reset_step_tree_stats();
        record_tree_fetch(7);
        let stats = take_step_tree_stats();
        assert_eq!(stats.fetches, 1, "one record SHALL count one fetch");
        assert_eq!(stats.min_nodes, 7, "single fetch min SHALL equal its count");
        assert_eq!(stats.max_nodes, 7, "single fetch max SHALL equal its count");
    }

    // 3. Multiple fetches track the running min and max across calls.
    #[test]
    fn multiple_fetches_track_min_and_max() {
        reset_step_tree_stats();
        record_tree_fetch(10);
        record_tree_fetch(3);
        record_tree_fetch(25);
        record_tree_fetch(8);
        let stats = take_step_tree_stats();
        assert_eq!(stats.fetches, 4, "four records SHALL count four fetches");
        assert_eq!(stats.min_nodes, 3, "min SHALL be the smallest node count seen");
        assert_eq!(stats.max_nodes, 25, "max SHALL be the largest node count seen");
    }

    // 4. A fetch count of zero nodes is a legitimate observation: min becomes 0
    //    while max reflects the larger value, and fetches is still counted.
    #[test]
    fn zero_node_fetch_is_recorded_as_min() {
        reset_step_tree_stats();
        record_tree_fetch(0);
        record_tree_fetch(5);
        let stats = take_step_tree_stats();
        assert_eq!(stats.fetches, 2, "two records SHALL count two fetches");
        assert_eq!(stats.min_nodes, 0, "a zero-node fetch SHALL set min to 0");
        assert_eq!(stats.max_nodes, 5, "max SHALL ignore the smaller zero fetch");
    }

    // 5. take_step_tree_stats resets state: a second take with no intervening
    //    record returns zeroed stats (the swap drained the accumulators).
    #[test]
    fn take_drains_state_for_next_take() {
        reset_step_tree_stats();
        record_tree_fetch(12);
        let first = take_step_tree_stats();
        assert_eq!(first.fetches, 1, "first take SHALL see the recorded fetch");
        let second = take_step_tree_stats();
        assert_eq!(second.fetches, 0, "take SHALL drain fetches for the next take");
        assert_eq!(second.min_nodes, 0, "drained min SHALL clamp to 0 when no fetches");
        assert_eq!(second.max_nodes, 0, "drained max SHALL be 0 when no fetches");
    }

    // 6. Recording after a take starts a fresh window (the take's swap reset
    //    the min sentinel back to u32::MAX), so the new min reflects only the
    //    post-take fetch.
    #[test]
    fn record_after_take_starts_fresh_window() {
        reset_step_tree_stats();
        record_tree_fetch(2);
        let _ = take_step_tree_stats();
        record_tree_fetch(99);
        let stats = take_step_tree_stats();
        assert_eq!(stats.fetches, 1, "post-take window SHALL count only the new fetch");
        assert_eq!(stats.min_nodes, 99, "post-take min SHALL be the new fetch count");
        assert_eq!(stats.max_nodes, 99, "post-take max SHALL be the new fetch count");
    }

    // 7. reset_step_tree_stats clears a partially-accumulated window so a later
    //    take reflects only fetches recorded after the reset.
    #[test]
    fn reset_clears_accumulated_window() {
        reset_step_tree_stats();
        record_tree_fetch(50);
        record_tree_fetch(60);
        reset_step_tree_stats();
        record_tree_fetch(4);
        let stats = take_step_tree_stats();
        assert_eq!(stats.fetches, 1, "reset SHALL drop pre-reset fetches");
        assert_eq!(stats.min_nodes, 4, "reset SHALL drop pre-reset min");
        assert_eq!(stats.max_nodes, 4, "reset SHALL drop pre-reset max");
    }

    // 8. fail_code! returns an Err whose chain carries the supplied FailureCode
    //    and formats the message arguments.
    #[test]
    fn fail_code_tags_error_with_code_and_message() {
        let err = fails_with(golem_events::FailureCode::FlowElementNotFound)
            .expect_err("fail_code! SHALL produce an Err");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::FlowElementNotFound),
            "fail_code! SHALL tag the error chain with the given code",
        );
        assert!(
            format!("{err:#}").contains("boom 42"),
            "fail_code! SHALL format its message arguments",
        );
    }

    // 9. fail_code! forwards whichever code it is handed rather than hardcoding
    //    one: each distinct input code round-trips to its own extracted tag.
    #[test]
    fn fail_code_preserves_distinct_codes() {
        // 9a. A spread of distinct variants, each of which SHALL come back out
        //     unchanged — guards against the macro pinning a single code.
        let codes = [
            golem_events::FailureCode::FlowStepTimeout,
            golem_events::FailureCode::FlowElementNotFound,
            golem_events::FailureCode::FlowAssertionMismatch,
        ];
        for code in codes {
            let err = fails_with(code).expect_err("fail_code! SHALL produce an Err");
            assert_eq!(
                golem_events::extract_code(&err),
                Some(code),
                "fail_code! SHALL carry the exact code it was given, not a hardcoded one",
            );
        }
    }
}
