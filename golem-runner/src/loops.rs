use std::collections::HashMap;

/// Default safety limit for total block entries across an entire flow.
pub const DEFAULT_MAX_ITERATIONS: u32 = 1000;

/// Tracks loop counters per block and enforces iteration limits.
///
/// Each block has an independent `_loop` counter that increments every time the
/// block is entered (first entry = 0, second = 1, etc.). A global safety limit
/// (`max_iterations`) caps the total number of block entries across the entire
/// flow to prevent infinite loops.
pub struct LoopTracker {
    counters: HashMap<String, u32>,
    total_entries: u32,
    max_iterations: u32,
}

impl LoopTracker {
    pub fn new(max_iterations: u32) -> Self {
        Self {
            counters: HashMap::new(),
            total_entries: 0,
            max_iterations,
        }
    }

    /// Record entry into a block. Returns the `_loop` counter value for this
    /// block (0-based: first entry returns 0, second returns 1, etc.).
    ///
    /// Returns an error if the global `max_iterations` limit is exceeded.
    pub fn enter_block(&mut self, block_name: &str) -> Result<u32, anyhow::Error> {
        self.total_entries += 1;
        if self.total_entries > self.max_iterations {
            anyhow::bail!(
                "Maximum iterations ({}) exceeded. Possible infinite loop at block '{}'",
                self.max_iterations,
                block_name
            );
        }
        let count = self.counters.entry(block_name.to_string()).or_insert(0);
        let loop_val = *count;
        *count += 1;
        Ok(loop_val)
    }

    /// Get current `_loop` value for a block without incrementing.
    /// Returns 0 for blocks that have never been entered.
    pub fn get_loop_count(&self, block_name: &str) -> u32 {
        self.counters.get(block_name).copied().unwrap_or(0)
    }

    /// Reset counter for a specific block.
    pub fn reset_block(&mut self, block_name: &str) {
        self.counters.remove(block_name);
    }

    /// Get the total number of block entries recorded so far.
    pub fn total_entries(&self) -> u32 {
        self.total_entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // 1. First entry returns _loop = 0
    // ---------------------------------------------------------------
    #[test]
    fn first_entry_returns_zero() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);
        let val = tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        assert_eq!(val, 0, "first entry SHALL return _loop = 0");
    }

    // ---------------------------------------------------------------
    // 2. Second entry returns _loop = 1
    // ---------------------------------------------------------------
    #[test]
    fn second_entry_returns_one() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);
        tracker
            .enter_block("block_a")
            .expect("first entry should succeed");
        let val = tracker
            .enter_block("block_a")
            .expect("second entry should succeed");
        assert_eq!(val, 1, "second entry SHALL return _loop = 1");
    }

    // ---------------------------------------------------------------
    // 3. Multiple entries increment correctly
    // ---------------------------------------------------------------
    #[test]
    fn multiple_entries_increment_correctly() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);
        for expected in 0..10 {
            let val = tracker
                .enter_block("loop_block")
                .expect("should not exceed max iterations");
            assert_eq!(
                val, expected,
                "entry {expected} should return _loop = {expected}"
            );
        }
    }

    // ---------------------------------------------------------------
    // 4. Different blocks have independent counters
    // ---------------------------------------------------------------
    #[test]
    fn different_blocks_have_independent_counters() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);

        // Enter block_a three times
        for _ in 0..3 {
            tracker
                .enter_block("block_a")
                .expect("should not exceed max iterations");
        }

        // Enter block_b once
        let val_b = tracker
            .enter_block("block_b")
            .expect("should not exceed max iterations");
        assert_eq!(val_b, 0, "block_b SHALL start at 0 independently");

        // block_a should continue from 3
        let val_a = tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        assert_eq!(val_a, 3, "block_a SHALL continue its own counter at 3");
    }

    // ---------------------------------------------------------------
    // 5. Max iterations exceeded returns error
    // ---------------------------------------------------------------
    #[test]
    fn max_iterations_exceeded_returns_error() {
        let mut tracker = LoopTracker::new(3);

        tracker.enter_block("a").expect("entry 1 should succeed");
        tracker.enter_block("a").expect("entry 2 should succeed");
        tracker.enter_block("a").expect("entry 3 should succeed");

        let result = tracker.enter_block("a");
        assert!(result.is_err(), "fourth entry SHALL exceed max_iterations=3");

        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("Maximum iterations (3) exceeded"),
            "error should mention the limit: {err_msg}"
        );
        assert!(
            err_msg.contains("block 'a'"),
            "error should mention the block name: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 6. Default max_iterations is 1000
    // ---------------------------------------------------------------
    #[test]
    fn default_max_iterations_is_1000() {
        assert_eq!(
            DEFAULT_MAX_ITERATIONS, 1000,
            "default max iterations should be 1000"
        );
    }

    // ---------------------------------------------------------------
    // 7. Custom max_iterations respected
    // ---------------------------------------------------------------
    #[test]
    fn custom_max_iterations_respected() {
        let mut tracker = LoopTracker::new(5);

        for _ in 0..5 {
            tracker
                .enter_block("block")
                .expect("should succeed within limit");
        }

        let result = tracker.enter_block("block");
        assert!(
            result.is_err(),
            "6th entry should exceed custom max_iterations=5"
        );
    }

    // ---------------------------------------------------------------
    // 8. get_loop_count returns current value
    // ---------------------------------------------------------------
    #[test]
    fn get_loop_count_returns_current_value() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);

        assert_eq!(
            tracker.get_loop_count("block_a"),
            0,
            "before any entry, count should be 0"
        );

        tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        assert_eq!(
            tracker.get_loop_count("block_a"),
            1,
            "after one entry, get_loop_count should return 1 (the next _loop value)"
        );

        tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        assert_eq!(
            tracker.get_loop_count("block_a"),
            2,
            "after two entries, get_loop_count should return 2"
        );
    }

    // ---------------------------------------------------------------
    // 9. reset_block clears counter
    // ---------------------------------------------------------------
    #[test]
    fn reset_block_clears_counter() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);

        // Enter block 5 times
        for _ in 0..5 {
            tracker
                .enter_block("block_a")
                .expect("should not exceed max iterations");
        }
        assert_eq!(tracker.get_loop_count("block_a"), 5);

        // Reset
        tracker.reset_block("block_a");
        assert_eq!(
            tracker.get_loop_count("block_a"),
            0,
            "after reset, count should be 0"
        );

        // Next entry should start from 0 again
        let val = tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        assert_eq!(val, 0, "after reset, first entry SHALL return _loop = 0");
    }

    // ---------------------------------------------------------------
    // 10. Total entries tracked across all blocks
    // ---------------------------------------------------------------
    #[test]
    fn total_entries_tracked_across_all_blocks() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);

        tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        tracker
            .enter_block("block_b")
            .expect("should not exceed max iterations");
        tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        tracker
            .enter_block("block_c")
            .expect("should not exceed max iterations");

        assert_eq!(
            tracker.total_entries(),
            4,
            "total entries should count all block entries across all blocks"
        );
    }

    // ---------------------------------------------------------------
    // 11. Block name that was never entered returns 0
    // ---------------------------------------------------------------
    #[test]
    fn never_entered_block_returns_zero() {
        let tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);
        assert_eq!(
            tracker.get_loop_count("nonexistent_block"),
            0,
            "a block that was never entered should return 0"
        );
    }

    // ---------------------------------------------------------------
    // 12. Max iterations boundary: exactly at limit succeeds
    // ---------------------------------------------------------------
    #[test]
    fn max_iterations_boundary_at_limit_succeeds() {
        let mut tracker = LoopTracker::new(2);

        let val1 = tracker.enter_block("a").expect("entry 1 should succeed");
        assert_eq!(val1, 0);

        let val2 = tracker.enter_block("a").expect("entry 2 should succeed (at limit)");
        assert_eq!(val2, 1);

        // Next entry should fail
        assert!(
            tracker.enter_block("a").is_err(),
            "entry beyond limit should fail"
        );
    }

    // ---------------------------------------------------------------
    // 13. Max iterations enforced across different blocks
    // ---------------------------------------------------------------
    #[test]
    fn max_iterations_enforced_across_different_blocks() {
        let mut tracker = LoopTracker::new(3);

        tracker.enter_block("a").expect("entry 1");
        tracker.enter_block("b").expect("entry 2");
        tracker.enter_block("c").expect("entry 3");

        // 4th entry, even to a new block, should fail
        let result = tracker.enter_block("d");
        assert!(
            result.is_err(),
            "total entries across all blocks should be capped by max_iterations"
        );
    }

    // ---------------------------------------------------------------
    // 14. Reset block does not affect total entries count
    // ---------------------------------------------------------------
    #[test]
    fn reset_block_does_not_affect_total_entries() {
        let mut tracker = LoopTracker::new(DEFAULT_MAX_ITERATIONS);

        tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");
        tracker
            .enter_block("block_a")
            .expect("should not exceed max iterations");

        assert_eq!(tracker.total_entries(), 2);

        tracker.reset_block("block_a");

        // Total entries should remain unchanged after reset
        assert_eq!(
            tracker.total_entries(),
            2,
            "reset_block should not reduce total_entries"
        );
    }

    // ---------------------------------------------------------------
    // 15. Max iterations of 1 allows exactly one entry
    // ---------------------------------------------------------------
    #[test]
    fn max_iterations_one_allows_single_entry() {
        let mut tracker = LoopTracker::new(1);

        let val = tracker.enter_block("only").expect("single entry should succeed");
        assert_eq!(val, 0);

        assert!(
            tracker.enter_block("only").is_err(),
            "second entry should fail with max_iterations=1"
        );
    }
}
