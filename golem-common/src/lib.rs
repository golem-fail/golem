//! Small, process-global primitives shared across the golem workspace.
//!
//! This is the crate every other golem crate sits on top of, so it stays
//! deliberately tiny: a global debug flag ([`set_debug`]/[`is_debug`]) that
//! gates verbose diagnostic output, [`command`]'s hermetic subprocess seam
//! ([`command::CommandRunner`], swapped for a [`command::FakeCommandRunner`]
//! in tests), and [`host_queue`]'s per-class semaphore registry that
//! serializes ops contending on a single host-wide resource (the `adb`
//! server, `CoreSimulatorService`) while leaving unrelated device ops to run
//! concurrently.

use std::sync::atomic::{AtomicBool, Ordering};

pub mod command;
pub mod host_queue;

static DEBUG: AtomicBool = AtomicBool::new(false);

/// Enable or disable the process-wide debug-output flag (e.g. `--debug` on
/// the CLI), checked via [`is_debug`] at call sites that emit extra diagnostics.
pub fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

/// Whether debug output is currently enabled; see [`set_debug`].
pub fn is_debug() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. Full debug-flag lifecycle in one test (the flag is a process-global
    //    atomic shared across all tests in a binary, so exercising it within a
    //    single test avoids depending on inter-test ordering): the default is
    //    false, set_debug(true) is observed by is_debug, set_debug(false)
    //    clears it, and the flag is idempotent under repeated identical sets.
    #[test]
    fn debug_flag_lifecycle() {
        // Restore on exit so this test does not leak state into others sharing
        // the same process (e.g. under `cargo test`'s threaded runner).
        let original = is_debug();

        set_debug(false);
        assert!(
            !is_debug(),
            "set_debug(false) SHALL make is_debug return false"
        );

        set_debug(true);
        assert!(
            is_debug(),
            "set_debug(true) SHALL make is_debug return true"
        );

        set_debug(true);
        assert!(is_debug(), "repeated set_debug(true) SHALL remain true");

        set_debug(false);
        assert!(
            !is_debug(),
            "set_debug(false) SHALL clear the flag back to false"
        );

        set_debug(original);
    }
}
