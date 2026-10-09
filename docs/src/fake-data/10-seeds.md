## Determinism and seeds

Generators draw from the run's random stream. Pass `--seed <N>` and every value
is **reproducible** — the same seed replays the same data, so assertions on
generated values stay stable. Without a seed, values are fresh each run, and the
seed actually used is reported so any run can be replayed.

**Time-based generators track "now" *and* reproduce.** `fake:timestamp` and a
card's expiry are anchored on a reference instant carried in the seed. A
no-`--seed` run anchors on the real current time; replaying the reported seed
reproduces the same dates. A small seed like `--seed 42` anchors at 2020 —
consistent, just not "now".
