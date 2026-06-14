use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG: AtomicBool = AtomicBool::new(false);

pub fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

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
        assert!(!is_debug(), "set_debug(false) SHALL make is_debug return false");

        set_debug(true);
        assert!(is_debug(), "set_debug(true) SHALL make is_debug return true");

        set_debug(true);
        assert!(is_debug(), "repeated set_debug(true) SHALL remain true");

        set_debug(false);
        assert!(!is_debug(), "set_debug(false) SHALL clear the flag back to false");

        set_debug(original);
    }
}
