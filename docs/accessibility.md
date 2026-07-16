# Accessibility auditing

*The mechanical a11y check on every run.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) · [CLI Reference](cli-reference.md)

Golem audits every flow for accessibility issues automatically — zero config, on
by default. After each block it inspects the **visible** UI tree (the same tree
your assertions judge), reports findings in the live run and every report format,
and — whenever it has an image to draw on (always at `strict`, and at other levels
when a recording frame or failure capture supplied one) — saves an **annotated
screenshot** marking each issue.

It's a fast, build-time signal — not a replacement for a manual audit or a real
assistive-technology pass — but it catches the common, mechanical problems
(tiny tap targets, unlabeled controls, low-contrast text) on every run.

## Contents

- [Levels](#levels)
- [What gets judged](#what-gets-judged)
- [Checks](#checks)
- [Confidence](#confidence)
- [Output](#output)
- [Reading the annotated screenshot](#reading-the-annotated-screenshot)
- [Notes & limitations](#notes--limitations)

## Levels

Set per flow with `[flow.options].a11y`, per project with `[options].a11y`, or
override everything from the CLI with `--a11y`.

| Level | What runs | Use |
|-------|-----------|-----|
| `off` | nothing | Disable auditing. |
| `critical` | all tree checks, no screenshot | Fast CI gate — catches the worst, never captures a screenshot. |
| `relaxed` *(default)* | all tree checks | Everyday default. Tree-only, so no screenshot cost. |
| `strict` | tree checks **+** a per-block screenshot for the contrast and pixel text-size checks, with WCAG-AAA warn bands | Thorough pass; produces the annotated screenshot. |

Only `strict` *forces* a screenshot — for the contrast check, the pixel
text-size pass, and the annotated image. The other levels never capture one
**solely** for a11y. But the audit reuses imagery already captured for another
reason, so the pixel checks (and an annotated image) often come for free:

- **From the recording.** When a recording is on (`--record` *or* `--trace`)
  and `ffmpeg` is installed, the audit pulls its image from the block recording
  — the frame at the moment the audited tree was captured — instead of taking a
  live screenshot. So `relaxed`/`critical` gain the contrast and pixel-text
  checks for free, and `strict` reuses the frame instead of re-shooting. A
  recording frame is **resolution-adaptive**: at full resolution it's
  indistinguishable from a live shot (and carries *no* tree↔screenshot timing
  drift, since frame and tree are taken at the same instant), so it's barely
  de-rated; if the device's encoder recorded at a reduced resolution, findings
  are de-rated more steeply (see [Confidence](#confidence)). If a frame is too
  low-resolution for the thorough checks, `strict` falls back to a clean live
  screenshot and `relaxed`/`critical` fall back to tree-only.
- **From a failure.** When a step **fails**, golem already captured a real
  `(screenshot, tree)` of the failing screen — the audit reuses it to report
  a11y findings on exactly the screen the run broke on. That's a live
  screenshot, so its findings keep full confidence.

Without `ffmpeg` or a recording, `strict` captures a fresh live screenshot and
the other levels stay tree-only.

```toml
[flow.options]
a11y = "relaxed"           # off | critical | relaxed | strict
a11y_max_errors = 0        # optional: fail the flow if cumulative errors exceed this
a11y_max_warnings = 20     # optional: fail the flow if cumulative warnings exceed this
a11y_min_confidence = 0.8  # optional: drop findings below this confidence (0–1)
```

```bash
golem run flow.test.toml --a11y strict                  # override every flow's level
golem run flow.test.toml --a11y strict --a11y-min-confidence 0   # …and surface every finding
```

By default findings are **warnings only** and never fail a run; set
`a11y_max_errors` / `a11y_max_warnings` to gate.

## What gets judged

- **Visible tree only.** A node is a subject of a check only if it's actually on
  screen — within the viewport, not clipped away by a scroll container, and not
  painted under an opaque overlay (a sticky header, a backdrop). Golem judges
  what a user can see.
- **The innermost actionable control.** Native platforms mark structural
  containers (windows, layout groups, web views) as clickable; Golem audits the
  real tap target — the innermost clickable node — not its wrappers.
- **dp-normalised sizes.** Size thresholds are in density-independent pixels, so
  a verdict is the same on an Android device (px) and an iOS device (points).
- **Disabled and oversized controls are exempt.** A disabled control (WCAG
  exempts inactive components) and a clickable that covers most of the screen
  (a backdrop / root tap surface, not a perceived control) are skipped.
- **Partially-clipped elements skip the size checks.** When a scroll container
  clips an element so only a sliver shows, its visible bounds are misleadingly
  tiny — measuring them would be a false "too small". So a clipped element is
  exempt from the size-dependent checks (touch target, contrast, pixel
  text-size). The size-independent checks (missing label, duplicate labels) still
  apply, and the box-height text check uses the element's full bounds, so it's
  clip-safe.

## Checks

| Check | Severity | Screenshot | Rule |
|-------|----------|------------|------|
| `missing_label` | Error | — | Actionable control with no text or accessibility label anywhere in its subtree. |
| `touch_target_too_small` | Error / Warning | — | Min dimension below the dp threshold (see below). |
| `text_too_small` | Warning | box: — · glyphs: `strict` | Two passes: the **box** height below the min dp (certain, all levels), plus a `strict` **pixel** pass estimating glyph size to catch small text in a tall/padded/multi-line box. See the note below. |
| `duplicate_labels` | Warning | — | Sibling controls sharing identical visible text — a screen reader can't tell them apart. |
| `overlapping_interactive` | Warning | — | Sibling controls with overlapping bounds (coincident / wrapper-enclosed pairs excluded). |
| `occluded_element` | Warning | — | Actionable control whose tap target is majority-covered by an overlay (from the hit-test's reachable points). Fully-covered controls are already dropped from the visible tree; this catches the *partial* case. |
| `low_contrast` | Error / Warning | `strict` | Text/background WCAG contrast below threshold. Heuristic — carries a confidence score. |

### Thresholds by level

| | `critical` | `relaxed` | `strict` |
|---|---|---|---|
| Touch target — **error** | `<24dp` | `<24dp` | `<44dp` |
| Touch target — **warning** | `24–32dp` | `24–44dp` | — |
| Text box height — **warning** | `<8dp` | `<10dp` | `<12dp` |
| Contrast — **error** | — | — | `<4.5:1` (normal), `<3:1` (large — font ≥18pt ≈ 24dp) |
| Contrast — **warning** | — | — | `4.5–7:1` / `3–4.5:1` (below AAA) |
| Occlusion — **warning** | `>50%` covered | `>50%` covered | `>25%` covered |

A finding's severity can differ by level — a 32dp target is a *warning* at
`relaxed` but an *error* at `strict`. That's intentional; run the level that
matches how strict you want to be.

`critical` is the lean worst-offenders gate: its lower `<8dp` text floor (and no
screenshot at all, so no contrast check) means it deliberately **misses more**
small-text and contrast issues than `relaxed`/`strict` — it's tuned to flag only
the egregious cases fast, not to be exhaustive.

## Confidence

Every finding carries a **confidence** (0–1) — how sure Golem is that the element
is non-compliant.

- **Deterministic checks are always `1.0`** — touch target, missing label,
  duplicate, overlap, occlusion, and the box-height check for `text_too_small`.
  These come straight from bounds, structure, and the hit-test, so there's
  nothing to be unsure about.
- **Heuristic checks score lower when the read is uncertain** — the two that
  measure pixels from the screenshot: contrast, and the `strict` pixel pass for
  `text_too_small` (estimating glyph size in a tall/padded box the box-height
  check can't see). Placeholder hint text, busy backgrounds, and borderline
  measurements pull the score down.
- **Recording-sourced frames are de-rated by resolution.** When a finding's
  pixels came from a recording frame rather than a live capture, its heuristic
  confidence is scaled down: negligibly at full resolution, more steeply the
  further the recording was downscaled below the device's native resolution
  (compression plus fewer pixels per glyph make the read less certain). The
  curve is convex — a drop near full resolution barely matters, a drop near the
  cutoff bites hard. Deterministic findings are never affected.

**Level defaults.** Each level sets a default `a11y_min_confidence`: `relaxed` and
`critical` default to **`1.0`** (they run no heuristic checks, so out of the box
there's zero heuristic noise), `strict` defaults to **`0.5`**. Precedence (highest
first): the CLI `--a11y-min-confidence` flag, then `[flow.options]`/`[options]
.a11y_min_confidence`, then the level default — set `0.0` to see everything, a
higher value to keep only confident findings.

## Output

Findings appear everywhere a run is reported, each with a **number** that matches
the marker drawn on the annotated screenshot:

- **Live run** — a per-block summary line `╰─ a11y: N error(s), M warning(s)`
  (a clickable link to the annotated PNG when one was captured); `--verbose`
  lists each finding: `1 [WRN] touch_target_too_small button "Menu" 32dp below recommended 44dp`.
  The flow's `PASS`/`FAIL` line and the suite `Summary` line each carry a rolled-up
  total (`· a11y: N error(s), M warning(s)`) so a CI run shows its a11y tally at a glance.
- **human / json / junit / toon** — the same findings, numbered, with counts.
  A confidence below `1.0` is shown alongside the finding (e.g. `(confidence
  0.70)` in human/junit, `c0.70` in toon) so a heuristic result is never mistaken
  for a certain one; `json` always carries `confidence` plus the `marker`,
  `check`, `severity`, and a compact `detail` (e.g. `32dp`, `3.5:1`).

## Reading the annotated screenshot

When a block has findings **and an image is available**, Golem saves an annotated
PNG to the run's screenshot directory (path shown on the live `a11y:` line). That
image is always present at `strict` (which forces a screenshot); at the other
levels the annotated PNG is written whenever the block recording — or a failure
capture — already supplied one. The visual language:

- **Rectangle** around each flagged element — **red = error**, **orange =
  warning** (warnings are drawn first, so red always wins where they overlap).
- **Numbered chip** at the top-left corner — the finding's number, matching the
  text output. When several single-element findings collide on a corner the
  chips cascade rightwards so every number stays visible.
- **Touch target** → an industrial **dimension line on the right** (or below, if
  width is the limiting axis) measuring the failing dimension, e.g. `32dp`.
- **Text too small** → a dimension line on the **left**, e.g. `11dp`.
- **Low contrast** → the measured ratio as a small semi-translucent token at the
  bottom-left, e.g. `3.5:1`.
- **Missing label** → a `?` at the bottom-right (the "what is this control?"
  marker), since there's nothing else to measure.
- **Duplicate labels** → a rectangle on **every** member of the group, joined by
  dashed connector lines, with the finding's number repeated on each segment.
- **Overlapping** → a rectangle on both elements with one number centred over
  the pair.
- **Occluded element** → a small **3×3 mini-map at the bottom-right** showing the
  hit-test's sampled zones: **covered** cells **solid**, **reachable** cells a
  **faint outline**, and untested zones left **blank** (the hit-test samples only
  1/3/5/9 points by size, so blank means "not tested", never "reachable"). You
  see *which* part of the tap target is unreachable — top, a corner, the lower
  half — not just that some of it is.

When two findings apply to one element they use different channels and stay
legible — e.g. a small low-contrast button shows a right-side dimension line
(touch target) *and* a bottom-left ratio token (contrast).

### Embedded metadata

The annotated PNG is **self-describing** — it carries the findings as text
chunks, so the image alone is enough to understand and reproduce a run when
shared standalone. The easiest way to read one is **`golem a11y-extract
<png>`**, which prints the findings in human form and the `golem run …` command
to replay that exact run (`--json` dumps the raw record for tooling). Any PNG
tool (`exiftool`, `pngcheck`) sees the same chunks:

- `Software` = `Golem` (`a11y-extract` requires this stamp and rejects foreign images)
- `Golem-Summary` = a human one-liner (flow, block, device, counts, seed, level)
- `Golem-Audit` = a JSON record: app / device / **platform** / flow / block /
  iteration / **seed** (to replay) / a11y level / image + viewport dimensions /
  counts, and every finding with its `marker`, `check`, `severity`, `message`,
  `detail`, `confidence`, and `bounds` **in screenshot pixels** (so a viewer can
  overlay them directly on the image), plus `related` rects for grouped findings.

## Notes & limitations

- **`text_too_small` runs in two passes.** First a *certain* box-height check
  (all levels, no screenshot): if the box is too short the text definitely is —
  no false positives. At **`strict`** a second *pixel* pass estimates the glyph
  size from the screenshot (cap-line/x-height of the tallest line) to catch
  small text inside a tall box — padding or multi-line — that the box-height
  pass can't see. The pixel pass is heuristic (`confidence < 1`) and the em
  estimate is biased *large*, so it favours the odd missed case over flagging
  normal text: the true font size can't always be known from pixels, and a
  false positive that drowns real findings in noise is worse than a false
  negative.
- **`low_contrast` is heuristic.** It isolates the glyph band and the ink colour
  from the screenshot; complex backgrounds (gradients, photos) are skipped as
  undetermined. Trust the confidence score and `a11y_min_confidence`.
- **Webview vs native occlusion.** In webviews Golem knows exactly what's painted
  on top (via the DOM hit-test), so occluded text is reliably skipped. Native
  occlusion is coarser.

See also: [test-structure.md](test-structure.md) (flow options) and
[cli-reference.md](cli-reference.md) (`--a11y`, `--a11y-min-confidence`).
