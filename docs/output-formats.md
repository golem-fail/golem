# Output Formats

*How the golem reports back.*

← [Back to README](../README.md)

Specify with `--output FORMAT`. Multiple formats can run simultaneously. Result
files are always written to `--output-dir` (default `.golem/results/`):
`results.json` and `results.toon` always, plus `results.xml` when `junit` is
requested.

## `human` (default)

Real-time colored output streamed to stderr. Shows step-by-step progress with timing, pass/fail symbols, and a suite summary.

```text
▶ tap.test
  ── tap_interactions ──
  [1][tap_interactions][0] tap on_text="+"
      ✓  [1200ms]
  [2][tap_interactions][1] assert_visible on_text="1" on_below="Counter"
      ✓  [320ms]

  ✓ PASSED  tap.test  [2.1s]
```

Step labels read as `[global_step][block_name][step_within_block]`. With data-driven tests or `for_each` iterations, the block name includes the iteration: `[3][login:0][1]`, `[6][login:1][1]`.

With `--verbose`, shows substeps and tree stats. The `{3 trees, 186~190 nodes}` suffix shows how many UI hierarchy fetches the step needed and the node count range across those fetches. Higher tree counts indicate retries or scroll iterations; changing node counts suggest the UI was updating.

Scroll substeps show the strategy number (1-5 per direction), swipe coordinates, and outcome. Strategies vary the swipe distance and position to handle different scroll contexts — strategy 1 is a full-page swipe, higher numbers try shorter or offset swipes to handle inner scrollable containers.
```text
  [3][tap_interactions][2] tap on_text="+"
      ∙ element_resolved "+" bounds=(48,161,43,36) tap=(69,179)
      ∙ tap (69,179)
      ✓  [2126ms] {3 trees, 186~190 nodes}
  [5][scroll_test][1] read on_right_of="Orientation:" auto_scroll
      ∙ [scroll] ↓ strategy 1 (540,1560)→(540,256) → page scrolled
      ∙ [scroll] ↓ strategy 2 (540,2160)→(540,840) → found at (550,459)
      ∙ element_resolved "Portrait" bounds=(200,459,80,18) tap=(240,468)
      ✓  [8234ms] {3 trees, 187~188 nodes}
```

## `json`

Structured JSON with suite summary, per-flow results, step details, substeps, and performance snapshots. Printed to stdout (also written to `{output-dir}/results.json`).

## `junit`

JUnit XML for CI systems (Jenkins, GitHub Actions, GitLab CI). Each flow maps to a `<testsuite>`, each step to a `<testcase>`. Printed to stdout (also written to `{output-dir}/results.xml`).

## `toon`

Token-optimized format for LLM analysis. ~40-60% smaller than human format.

```text
S:tap_test d:450 seed:847291036
 +tap:+ 45 t:3/142
 +assert_visible:1 120
R:PASS 2/0/0
```
