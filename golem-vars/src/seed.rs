use chrono::{DateTime, Duration, Utc};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Bits of the 64-bit seed reserved for the time bucket (high bits); the
/// remaining bits carry randomness.
const TIME_BITS: u32 = 20;
/// Low bits of the seed used for randomness (`64 - TIME_BITS`).
const RANDOM_BITS: u32 = 64 - TIME_BITS;
/// Largest representable time bucket (`2^TIME_BITS - 1`).
const MAX_BUCKET: u64 = (1 << TIME_BITS) - 1;
/// Mask selecting the random low bits.
const RANDOM_MASK: u64 = (1 << RANDOM_BITS) - 1;
/// One time bucket = 4 hours, in seconds. 20 bits of 4h buckets ≈ 478 years
/// of headroom from the epoch — finer than any "within last month/year" use
/// needs, with plenty of future-precision margin.
const BUCKET_SECS: i64 = 4 * 3600;
/// Unix seconds for the bucket epoch, 2020-01-01T00:00:00Z. Counting buckets
/// from a recent epoch (not the Unix epoch) means a small or hand-typed seed —
/// e.g. `--seed 42`, whose high bits are 0 — decodes to 2020, not 1970: a
/// sane, consistent anchor rather than a confusing one.
const EPOCH_UNIX: i64 = 1_577_836_800;

/// The bucket epoch as a `DateTime`.
fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(EPOCH_UNIX, 0).unwrap_or_default()
}

/// Decode the run's reference instant ("now") from a seed's high bits.
fn anchor_from_seed(seed: u64) -> DateTime<Utc> {
    let bucket = (seed >> RANDOM_BITS) as i64;
    epoch() + Duration::seconds(bucket * BUCKET_SECS)
}

/// Pack a fresh seed whose high bits encode the current 4h bucket since the
/// epoch and whose low bits are random. This is the only place wall-clock
/// "now" is read; everything downstream is a pure function of the seed, so a
/// recorded seed replays bit-for-bit *including* its date anchor.
fn pack_current_seed() -> u64 {
    let secs = (Utc::now() - epoch()).num_seconds().max(0);
    let bucket = ((secs / BUCKET_SECS) as u64).min(MAX_BUCKET);
    let random_low = rand::random::<u64>() & RANDOM_MASK;
    (bucket << RANDOM_BITS) | random_low
}

/// Deterministic RNG for fake data, carrying the run's date **anchor** packed
/// into the seed's high bits (UUIDv7-style, but a `u64`).
///
/// Anchoring time-based generators (`timestamp`, card expiry, DOB) on this
/// instead of `Utc::now()` keeps them seed-reproducible while a no-`--seed`
/// run still tracks real "now" — the anchor rides inside the seed, so it can't
/// drift between runs of the same seed. `FakeRng` implements [`RngCore`], so
/// it is accepted anywhere an `impl Rng` is, leaving leaf generators generic.
pub struct FakeRng {
    seed: u64,
    anchor: DateTime<Utc>,
    inner: ChaCha8Rng,
}

impl FakeRng {
    /// Build from an explicit seed; the anchor is decoded from its high bits.
    pub fn from_seed(seed: u64) -> Self {
        Self {
            seed,
            anchor: anchor_from_seed(seed),
            inner: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// Build from an optional user seed: `Some(s)` is used verbatim (for
    /// replay); `None` packs the current time bucket with random low bits (a
    /// normal run). Either way the returned [`seed`](Self::seed) reproduces it.
    pub fn from_optional_seed(seed: Option<u64>) -> Self {
        Self::from_seed(seed.unwrap_or_else(pack_current_seed))
    }

    /// The seed value (print this for reproduction; pass it back via `--seed`).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The run's reference instant — time-based generators anchor on this.
    pub fn anchor(&self) -> DateTime<Utc> {
        self.anchor
    }

    /// A child RNG for per-device / per-sub-flow independence: a fresh seed
    /// drawn from this stream, but the **same anchor**. Child seeds are random
    /// (their high bits don't encode the bucket), so the anchor must be carried
    /// across rather than re-decoded — the run has one "now" for every flow.
    pub fn child(&mut self) -> FakeRng {
        let child_seed = self.inner.next_u64();
        FakeRng {
            seed: child_seed,
            anchor: self.anchor,
            inner: ChaCha8Rng::seed_from_u64(child_seed),
        }
    }
}

impl RngCore for FakeRng {
    fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.inner.fill_bytes(dest)
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.inner.try_fill_bytes(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::generate_simple;
    use crate::GeneratorDef;
    use chrono::{Datelike, Timelike};
    use rand::Rng;
    use std::collections::HashMap;

    // 1. Same seed produces same RNG sequence
    #[test]
    fn same_seed_produces_same_sequence() {
        let mut sm1 = FakeRng::from_seed(42);
        let mut sm2 = FakeRng::from_seed(42);

        let vals1: Vec<u64> = (0..10).map(|_| sm1.gen()).collect();
        let vals2: Vec<u64> = (0..10).map(|_| sm2.gen()).collect();

        assert_eq!(vals1, vals2, "same seed SHALL produce identical sequences");
    }

    // 2. Different seeds produce different sequences
    #[test]
    fn different_seeds_produce_different_sequences() {
        let mut sm1 = FakeRng::from_seed(42);
        let mut sm2 = FakeRng::from_seed(99);

        let vals1: Vec<u64> = (0..10).map(|_| sm1.gen()).collect();
        let vals2: Vec<u64> = (0..10).map(|_| sm2.gen()).collect();

        assert_ne!(
            vals1, vals2,
            "different seeds SHALL produce different sequences"
        );
    }

    // 3. child() creates independent RNG, reproducible across same-seed parents
    #[test]
    fn child_creates_independent_rng() {
        let mut parent = FakeRng::from_seed(42);
        let mut child = parent.child();

        let parent_val: u64 = parent.gen();
        let child_val: u64 = child.gen();
        assert_ne!(
            parent_val, child_val,
            "parent and child should produce different values"
        );

        let mut parent2 = FakeRng::from_seed(42);
        let mut child2 = parent2.child();
        let parent2_val: u64 = parent2.gen();
        let child2_val: u64 = child2.gen();

        assert_eq!(
            parent_val, parent2_val,
            "parent sequences SHALL match with same seed"
        );
        assert_eq!(
            child_val, child2_val,
            "child sequences SHALL match with same parent seed"
        );
    }

    // 4. seed() returns the original seed
    #[test]
    fn seed_returns_original_seed() {
        assert_eq!(FakeRng::from_seed(12345).seed(), 12345);
        assert_eq!(FakeRng::from_seed(0).seed(), 0);
        assert_eq!(FakeRng::from_seed(u64::MAX).seed(), u64::MAX);
    }

    // 5. from_optional_seed(None) generates a reproducible seed
    #[test]
    fn auto_seed_is_reproducible() {
        let mut sm = FakeRng::from_optional_seed(None);
        let seed = sm.seed();
        let from_auto: Vec<u64> = (0..5).map(|_| sm.gen()).collect();

        let mut replay = FakeRng::from_seed(seed);
        let from_replay: Vec<u64> = (0..5).map(|_| replay.gen()).collect();

        assert_eq!(
            from_auto, from_replay,
            "an auto-seeded instance SHALL replay from its reported seed"
        );
    }

    // 6. Integration: same seed → same generated value
    #[test]
    fn integration_same_seed_same_generated_values() {
        let def = GeneratorDef {
            name: "email".to_string(),
            params: HashMap::new(),
            positional: Vec::new(),
        };
        let mut sm1 = FakeRng::from_seed(77);
        let mut sm2 = FakeRng::from_seed(77);

        let val1 = generate_simple(&def, &mut sm1).expect("should generate");
        let val2 = generate_simple(&def, &mut sm2).expect("should generate");

        assert_eq!(val1, val2, "same seed SHALL produce same generated value");
    }

    // 7. Integration: two generate calls advance the RNG
    #[test]
    fn integration_successive_generate_calls_differ() {
        let def = GeneratorDef {
            name: "email".to_string(),
            params: HashMap::new(),
            positional: Vec::new(),
        };
        let mut sm = FakeRng::from_seed(77);

        let val1 = generate_simple(&def, &mut sm).expect("should generate");
        let val2 = generate_simple(&def, &mut sm).expect("should generate");

        assert_ne!(
            val1, val2,
            "successive generate calls should produce different values"
        );
    }

    // 8. Two successive children from one parent differ
    #[test]
    fn successive_children_differ_from_one_parent() {
        let mut parent = FakeRng::from_seed(42);
        let mut child_a = parent.child();
        let mut child_b = parent.child();

        let a_val: u64 = child_a.gen();
        let b_val: u64 = child_b.gen();

        assert_ne!(
            a_val, b_val,
            "two children from one parent SHALL have different sequences"
        );
    }

    // 9. child() seed is reproducible across same-seed parents
    #[test]
    fn child_seed_reproducible_across_parents() {
        let mut parent1 = FakeRng::from_seed(123);
        let mut parent2 = FakeRng::from_seed(123);

        assert_eq!(
            parent1.child().seed(),
            parent2.child().seed(),
            "child seed SHALL match for parents with the same seed"
        );
    }

    // --- Anchor (seed-packed date baseline) ---

    // 10. A bare/small seed (high bits 0) anchors at the 2020 epoch, not 1970.
    #[test]
    fn small_seed_anchors_at_epoch() {
        let anchor = FakeRng::from_seed(42).anchor();
        assert_eq!(anchor, epoch(), "seed<2^44 SHALL anchor at the epoch");
        assert_eq!(anchor.year(), 2020);
    }

    // 11. The anchor is purely seed-derived: same seed → same anchor.
    #[test]
    fn anchor_is_seed_reproducible() {
        let seed = pack_current_seed();
        assert_eq!(
            FakeRng::from_seed(seed).anchor(),
            FakeRng::from_seed(seed).anchor(),
            "same seed SHALL decode the same anchor"
        );
    }

    // 12. An auto-generated seed anchors near real "now" (within one bucket).
    #[test]
    fn auto_seed_anchors_near_now() {
        let anchor = FakeRng::from_optional_seed(None).anchor();
        let now = Utc::now();
        let delta = (now - anchor).num_seconds();
        assert!(
            (0..=BUCKET_SECS).contains(&delta),
            "auto anchor SHALL be within one 4h bucket of now, delta={delta}s"
        );
    }

    // 13. child() carries the parent anchor (the run has one "now").
    #[test]
    fn child_preserves_anchor() {
        let mut parent = FakeRng::from_seed(pack_current_seed());
        let parent_anchor = parent.anchor();
        let child = parent.child();
        assert_eq!(
            child.anchor(),
            parent_anchor,
            "child SHALL inherit the parent's anchor, not re-decode from its random seed"
        );
    }

    // 14. Time bucket round-trips through pack/decode at 4h resolution.
    #[test]
    fn bucket_round_trips_at_4h_resolution() {
        let anchor = FakeRng::from_optional_seed(None).anchor();
        // Anchor SHALL land exactly on a 4h boundary from the epoch.
        assert_eq!(anchor.minute(), 0);
        assert_eq!(anchor.second(), 0);
        assert_eq!(anchor.hour() % 4, 0);
    }
}
