## Occlusion-aware tapping

The visible tree tells golem what's *clipped or off-screen*, but not what's *covered*
by something painted on top (a sticky header, a `z-index` overlay). So golem
**hit-tests** the target before tapping and **routes around** an occluder: a plain
`tap` lands on the first clear sample point (centre → arms → corners), so a button
whose centre sits under a sticky header still gets hit on a clear edge. The routed
coordinate shows in the `--verbose` `element_resolved` substep (`tap=(x,y)`).

Two guarantees:

- **It never blocks.** Occlusion is a heuristic — golem always attempts the tap; if
  no sampled point is clear it falls back to the centre. Treat a reported occlusion
  as *"may be covered"*, not a hard failure.
- **Offsets stay centre-relative.** `x`/`y` offsets are always measured from the
  element's geometric centre, never the routed point — so they stay predictable
  regardless of what's covering the element.

This detects layout/paint occlusion only — an element under the OS status bar is a
separate, system-level concern. For *how* the hit-test computes paint order on each
platform, see [Architecture → occlusion & hit-testing](architecture.md#occlusion--hit-testing).
