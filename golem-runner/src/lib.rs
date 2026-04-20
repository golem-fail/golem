// golem-runner: test execution orchestrator

use std::sync::atomic::{AtomicU32, Ordering};

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
pub mod loops;
pub mod policy;
pub mod resolution;
pub mod scroll;
pub mod subflow;
pub mod parallel;
pub mod for_each;
pub mod perf;
pub mod teardown;
