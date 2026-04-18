// golem-runner: test execution orchestrator

use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Enable verbose logging for scroll, swipe, and other low-level operations.
pub fn set_verbose(enabled: bool) {
    VERBOSE.store(enabled, Ordering::Relaxed);
}

/// Check if verbose logging is enabled.
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
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
