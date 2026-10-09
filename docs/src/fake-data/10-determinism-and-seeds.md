## Determinism and seeds

Generators draw from the run's random stream. Pass `--seed <N>` and every value
is **reproducible** — the same seed replays the same data, so assertions on
generated values stay stable. Without a seed, values are fresh each run, and the
seed actually used is reported so any run can be replayed.

**Time-based generators track "now" *and* reproduce.** `fake:timestamp` and a
card's expiry are anchored on a reference instant packed into the seed's high
bits (a 4-hour bucket since 2020). A no-`--seed` run anchors on the real current
time; replaying the reported seed reproduces the same dates bit-for-bit, because
the anchor rides inside the seed. (A hand-typed small seed like `--seed 42`
anchors at 2020 — consistent, just not "now".)
