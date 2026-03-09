use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Manages deterministic random number generation for fake data.
///
/// Uses ChaCha8Rng for fast, reproducible PRNG. A known seed can be provided
/// for reproducibility, or a random seed can be generated for normal runs.
pub struct SeedManager {
    seed: u64,
    rng: ChaCha8Rng,
}

impl SeedManager {
    /// Create with a specific seed (for reproducibility).
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// Create with a random seed (for normal runs).
    /// Returns the seed so it can be printed for reproduction.
    pub fn random() -> Self {
        let seed: u64 = rand::random();
        Self::new(seed)
    }

    /// Get the seed value (for printing in output).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Get a mutable reference to the RNG for generating values.
    pub fn rng(&mut self) -> &mut ChaCha8Rng {
        &mut self.rng
    }

    /// Create a child RNG (for per-device or per-fixture independence).
    /// Uses the parent RNG to derive a new seed.
    pub fn child(&mut self) -> SeedManager {
        use rand::Rng;
        let child_seed: u64 = self.rng.gen();
        SeedManager::new(child_seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::generate_simple;
    use crate::GeneratorDef;
    use rand::Rng;
    use std::collections::HashMap;

    // 1. Same seed produces same RNG sequence
    #[test]
    fn same_seed_produces_same_sequence() {
        let mut sm1 = SeedManager::new(42);
        let mut sm2 = SeedManager::new(42);

        let vals1: Vec<u64> = (0..10).map(|_| sm1.rng().gen()).collect();
        let vals2: Vec<u64> = (0..10).map(|_| sm2.rng().gen()).collect();

        assert_eq!(vals1, vals2, "same seed should produce identical sequences");
    }

    // 2. Different seeds produce different sequences
    #[test]
    fn different_seeds_produce_different_sequences() {
        let mut sm1 = SeedManager::new(42);
        let mut sm2 = SeedManager::new(99);

        let vals1: Vec<u64> = (0..10).map(|_| sm1.rng().gen()).collect();
        let vals2: Vec<u64> = (0..10).map(|_| sm2.rng().gen()).collect();

        assert_ne!(vals1, vals2, "different seeds should produce different sequences");
    }

    // 3. child() creates independent RNG
    #[test]
    fn child_creates_independent_rng() {
        let mut parent = SeedManager::new(42);
        let mut child = parent.child();

        // Parent and child should produce different sequences
        let parent_val: u64 = parent.rng().gen();
        let child_val: u64 = child.rng().gen();

        assert_ne!(
            parent_val, child_val,
            "parent and child should produce different values"
        );

        // Two children from the same parent seed should be reproducible
        let mut parent2 = SeedManager::new(42);
        let mut child2 = parent2.child();
        // Reset parent2 to get same state
        let parent2_val: u64 = parent2.rng().gen();
        let child2_val: u64 = child2.rng().gen();

        assert_eq!(parent_val, parent2_val, "parent sequences should match with same seed");
        assert_eq!(child_val, child2_val, "child sequences should match with same parent seed");
    }

    // 4. seed() returns the original seed
    #[test]
    fn seed_returns_original_seed() {
        let sm = SeedManager::new(12345);
        assert_eq!(sm.seed(), 12345);

        let sm2 = SeedManager::new(0);
        assert_eq!(sm2.seed(), 0);

        let sm3 = SeedManager::new(u64::MAX);
        assert_eq!(sm3.seed(), u64::MAX);
    }

    // 5. random() generates a valid seed
    #[test]
    fn random_generates_valid_seed() {
        let sm = SeedManager::random();
        // The seed should be retrievable and the RNG should work
        let seed = sm.seed();
        // Verify we can recreate the same sequence from the reported seed
        let mut sm_replay = SeedManager::new(seed);
        let mut sm_original = SeedManager::new(seed);

        let vals_original: Vec<u64> = (0..5).map(|_| sm_original.rng().gen()).collect();
        let vals_replay: Vec<u64> = (0..5).map(|_| sm_replay.rng().gen()).collect();

        assert_eq!(
            vals_original, vals_replay,
            "random seed should be reproducible when reused"
        );
    }

    // 6. Integration: SeedManager + generate_simple produce same values with same seed
    #[test]
    fn integration_same_seed_same_generated_values() {
        let def = GeneratorDef {
            name: "first_name".to_string(),
            params: HashMap::new(),
        };

        let mut sm1 = SeedManager::new(77);
        let mut sm2 = SeedManager::new(77);

        let val1 = generate_simple(&def, sm1.rng()).expect("should generate");
        let val2 = generate_simple(&def, sm2.rng()).expect("should generate");

        assert_eq!(val1, val2, "same seed should produce same generated value");
    }

    // 7. Integration: two generate_simple calls advance RNG (different values)
    #[test]
    fn integration_successive_generate_calls_differ() {
        let def = GeneratorDef {
            name: "email".to_string(),
            params: HashMap::new(),
        };

        let mut sm = SeedManager::new(77);

        let val1 = generate_simple(&def, sm.rng()).expect("should generate");
        let val2 = generate_simple(&def, sm.rng()).expect("should generate");

        assert_ne!(
            val1, val2,
            "successive generate calls should produce different values"
        );
    }
}
