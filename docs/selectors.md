# Selectors

*How golem finds the element a step acts on.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) · [Actions Reference](actions-reference.md) · [Architecture: visibility model](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)

A selector describes *which* on-screen element a step targets. golem resolves it
against the **visible tree** — the elements a human can actually see (clipped to
ancestor containers; see the [visibility model](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)).
The same selector grammar is used everywhere an element is named: `tap`,
`assert_visible`, `read`, `scroll`'s `to`/`within`, swipe points, etc.

## Two syntaxes

**Flat** (`on_*` fields) for simple cases:

```toml
{ action = "tap", on_text = "Submit", on_below = "Counter" }
```

**Grouped** (`on = { … }`, also `to = { … }` / `within = { … }`) for anything
with traits, containment, or nested anchors:

```toml
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }
```

The grouped form is required for `traits`, `contains`, `inside`, and nested
anchors; the flat form covers `text`/`accessibility_label`/`index`/state/the four
directionals.

## Core selectors

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_text` | `text` | Visible text. Glob (`*`, `?`), case-insensitive, anchored (full-string — use globs for partial: `"Item *"`, `"*@*"`). |
| `on_accessibility_label` | `accessibility_label` | The element's accessibility identifier / aria-label. **See the guidance below — prefer `text`.** |
| `on_index` | `index` | The Nth match (0-based) after all other filters. |

### Prefer visible `text`; use `accessibility_label` sparingly

golem's premise is **testing like a human** — a human reads and taps *visible
text*, not an accessibility identifier they can't see. So default to `on_text`
(or a positional/`contains` selector). Reach for `on_accessibility_label` only
when:

1. **You are explicitly testing the accessibility label itself** — e.g.
   validating screen-reader semantics / a11y compliance. Here the label *is* the
   thing under test.
2. **As a throwaway shortcut** to *navigate* to the part you actually want to
   test, when the element you're tapping isn't itself the subject of the
   assertion (e.g. opening a menu by its stable `menu-toggle` id so you can get
   to the screen you care about). You're not testing the label, just using it to
   get somewhere.

Outside those cases, an `accessibility_label` selector tests something the user
never perceives, and silently passes even if the visible text is wrong. When in
doubt, use `text`.

## State filters

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_enabled` | `enabled` | Enabled state (`true`/`false`). |
| `on_checked` | `checked` | Checked/selected state (`true`/`false`). |
| `on_clickable` | `clickable` | Clickability. |

## Traits

Computed predicates on an element's geometry and content. All listed traits in a
selector must hold (AND). Traits are coordinate/content-derived and
cross-platform — they don't encode platform element types.

```toml
{ action = "assert_visible", on = { text = "Submit", traits = ["button", "wide"] } }
```

| Trait | True when |
|-------|-----------|
| `button` | Element type is a button or link. |
| `has_text` / `text` | Has non-empty text. |
| `no_text` | Has no text. |
| `short_text` | Text length 1–10. |
| `long_text` | Text length > 50. |
| `square` | Width/height ratio between 0.8 and 1.2. |
| `wide` | Width > 2 × height. |
| `tall` | Height > 2 × width. |
| `small` | Area < 2 500 px². |
| `large` | Area > 100 000 px². |

(`golem-element/src/selector.rs` is the source of truth for thresholds.)

> **Note:** `small`/`large` threshold raw bounds area, whose units differ by
> platform (Android device px vs iOS points), so the same element may be `large`
> on one platform and not the other. Avoid them in cross-platform assertions
> until they're density-normalized (roadmap). The ratio traits (`square`/`wide`/
> `tall`) are dimensionless and unaffected.

## Relational (positional) selectors

Locate an element by its position relative to a visible **anchor**:

| Selector | Grouped key | Keeps elements… |
|----------|-------------|-----------------|
| `on_below` | `below` | below the anchor's bottom |
| `on_above` | `above` | above the anchor's top |
| `on_right_of` | `right_of` | right of the anchor's right edge |
| `on_left_of` | `left_of` | left of the anchor's left edge |

```toml
{ action = "assert_visible", on_text = "2", on_below = "Counter" }
```

Two rules make these behave the way a human reads layout:

- **Cross-axis overlap is required.** `below`/`above` also require the candidate
  to **horizontally overlap** the anchor; `left_of`/`right_of` require **vertical
  overlap**. So "below the heading" means below *and in the heading's column* —
  an element in another column (e.g. a two-column tablet layout) is not matched.
  A full-width anchor overlaps everything, so this is invisible in the common
  case and only constrains narrow anchors. (Threshold: any positive overlap.)
- **Nearest-first.** Among matches, the one closest to the anchor (by gap along
  the relation's axis) comes first.

The anchor must be **on-screen**. If it exists but is scrolled off, the
relational match is treated as unresolved (empty) — which is the signal `within`
uses to scroll the anchor into view first.

## Geometric containment: `contains` / `inside`

Select by spatial nesting — coordinate-based, *not* DOM structure (golem
deliberately does not expose parent/child tree queries; a human perceives
positions, not the document tree).

| Grouped key | Keeps elements whose bounds… |
|-------------|------------------------------|
| `contains` | **fully enclose** the anchor (the box *around* X) |
| `inside` | are **fully enclosed by** the anchor (things *within* a region) |

```toml
# the container that holds "Item 0" (smallest such box)
{ action = "assert_visible", on = { contains = { text = "Item 0" } } }
# an item inside a labelled region
{ action = "assert_visible", on = { text = "Item 0", inside = { accessibility_label = "section-scroll-list" } } }
```

`contains` excludes the anchor itself (an element trivially contains itself) and
coincident zero-margin wrappers, and resolves **smallest-enclosing first**.

> **Caveat — `contains` is for *selection*, not picking a *scroll* container.**
> The smallest box enclosing a *single* item can be a per-item wrapper, not the
> scrollable list (and this differs across platforms). To scroll inside a list,
> prefer `within = { below = "<heading>" }`. (A size trait — e.g.
> `contains = "Item 0", traits = ["large"]` — can disambiguate toward the
> scrollable container; see the roadmap.)

## Nesting and chaining

**Nested anchors** — a relational/containment anchor can itself be a full
selector group (one level), not just bare text:

```toml
{ action = "tap", on = { text = "Left", below = { text = "Nested Layout", traits = ["has_text"] } } }
```

**Chaining predicates** — every key in a group is combined with AND. The full
resolution order is:

1. **Match** own-criteria (`text`/`accessibility_label`/state/`traits`) across the
   visible tree, in tree pre-order.
2. **Filter** by each relational/containment predicate (set intersection):
   directional = half-plane **and** cross-axis overlap; containment = full
   enclosure.
3. **Sort** the survivors: containment tightest-first → proximity nearest-first
   (primary-axis gap) → **tree pre-order** as a deterministic tie-break.
4. Apply `index` (0-based) and take the result.

Genuine ties (e.g. a row of equal-distance icons under a full-width heading)
resolve by pre-order — golem does **not** guess; disambiguate with `index` or an
extra predicate. The pre-order tie-break also keeps `--seed` replay deterministic.

## Occlusion-aware tapping

The visible tree (via IntersectionObserver) tells golem what's *clipped/off-screen*,
but not what's *covered* by something painted on top (a `position: sticky` header,
a `z-index` overlay). For webview targets, golem additionally **hit-tests** sample
points within the visible bounds (`document.elementFromPoint`) and:

- **Routes around DOM occluders.** A plain `tap` lands on the first occlusion-clear
  sample point (centre → arms → corners), so tapping a button whose centre is under
  a sticky header still hits the button (a clear edge), not the header. The routed
  coordinate shows in the `--verbose` `element_resolved` substep (`tap=(x,y)`).
- **Never blocks.** Occlusion is a heuristic — golem always attempts the tap (if no
  sampled point is clear it falls back to the centre).
- **Offsets stay centre-relative.** `x`/`y` offsets are always measured from the
  element's geometric centre, never the occlusion-routed point — so they remain
  predictable regardless of what's covering the element.

Notes: this is **webview-only** (native trees have no `elementFromPoint`) and detects
**DOM** occlusion only — an element under the OS status bar (system-level) is a
separate concern. Surfacing occlusion as a *warning/error* (e.g. fully-covered or an
offset on a covered target) is deferred to the planned accessibility audit, whose
severity model is the right home for it — not a per-step tag.

## `within` (scoping a scroll)

`scroll`'s `within = { … }` names the region to scroll inside. It uses the same
selector grammar; the robust idiom for an inner list is a heading-relative
anchor:

```toml
{ action = "scroll", to = { text = "Item 45" }, within = { below = "Scroll List" } }
```

See [Actions Reference → scroll](actions-reference.md) for the full action.
