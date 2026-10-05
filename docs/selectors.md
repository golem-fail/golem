# Selectors

*How golem finds the element a step acts on.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) · [Actions Reference](actions-reference.md) · [Architecture: visibility model](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)

A selector describes *which* on-screen element a step targets. golem resolves it
against the **visible tree** — the elements a human can actually see (clipped to
ancestor containers; see the [visibility model](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)).
The same selector grammar is used everywhere an element is named: `tap`,
`assert_visible`, `read`, `scroll`'s `to`/`within`, swipe points, etc.

## Contents

- [Two syntaxes](#two-syntaxes)
- [Core selectors](#core-selectors)
- [State filters](#state-filters)
- [Traits](#traits)
- [Relational (positional) selectors](#relational-positional-selectors)
- [Geometric containment: `contains` / `inside`](#geometric-containment-contains--inside)
- [Nesting and chaining](#nesting-and-chaining)
- [Occlusion-aware tapping](#occlusion-aware-tapping)
- [`within` (scoping a scroll)](#within-scoping-a-scroll)
- [Canvas-rendered UI (Compose, Compose Multiplatform, Flutter)](#canvas-rendered-ui-compose-compose-multiplatform-flutter)

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

### `min_matches` — the container of *repeated* items

The smallest box enclosing a *single* item is often a per-item wrapper (a
`<li>`, a list cell), not the scrollable list one level up. To target the
**container of several repeated items**, give the `contains` group form a
`min_matches`:

```toml
# the smallest element that encloses ≥2 "Row *" matches — i.e. the list,
# not a single row's wrapper. The idiomatic way to scope a scroll to a list:
{ action = "scroll", to = { text = "Row 45" }, within = { contains = { text = "Row *", min_matches = 2 } } }
```

Semantics: *the smallest visible element whose bounds enclose ≥ `min_matches`
elements matching the anchor.* A human recognises a list by **repetition**
(several similar items grouped), not by invisible scrollability — so this keeps
`contains` purely about what's visible. It counts only **visible** matches
(off-screen items are filtered), so the result is the scroll region's on-screen
box. `min_matches` defaults to `1` (today's smallest-single-enclosing
behaviour) and must be `1`–`100` (a larger value is rejected at parse time;
2–3 is all you ever need). `min_matches` is valid **only** on `contains` — it is
meaningless, and unwritable, elsewhere.

> If a list is so short that only one item is visible, `min_matches = 2` can't
> resolve it — but neither could a human see it's a scrollable list. Make the
> list taller, or fall back to `within = { below = "<heading>" }`.

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

## `within` (scoping a scroll)

`scroll`'s `within = { … }` names the region to scroll inside. It uses the same
selector grammar. Two robust idioms for an inner list:

```toml
# 1. heading-relative — scope to what's below a heading
{ action = "scroll", to = { text = "Item 45" }, within = { below = "Scroll List" } }

# 2. repeated-item container — scope to the box holding ≥2 matching items
#    (use when items are wrapped, e.g. <li>, and `below` isn't convenient)
{ action = "scroll", to = { text = "Row 45" }, within = { contains = { text = "Row *", min_matches = 2 } } }
```

See [`min_matches`](#min_matches--the-container-of-repeated-items) above and
[Actions Reference → scroll](actions-reference.md) for the full action.

## Canvas-rendered UI (Compose, Compose Multiplatform, Flutter)

Jetpack Compose, Compose Multiplatform and Flutter draw the whole UI into one
platform view. They do not appear to golem as one opaque leaf, the way a WebView
without enrichment does. Each framework copies its semantics tree into the
platform accessibility tree, so golem sees one node for each widget. Text
selectors, relational selectors and viewport filtering work on these nodes.

What changes is *which* annotation reaches golem, and how coarse the tree is.
The Compose facts below come from Jetpack Compose (`test-app-b`) and Compose
Multiplatform 1.11 (`test-app-k`), on Android API 36 and iOS 26. The Flutter
facts come from the Flutter documentation. They are not yet checked on a device
(see #72).

### Version floors

- **Compose Multiplatform 1.8.0 or later on iOS.** From 1.8.0 the Compose
  accessibility tree syncs to iOS automatically. Version 1.7.3 and earlier
  needed `AccessibilitySyncOptions`, which 1.8.0 removed.
- **Flutter 3.19 or later** for `Semantics(identifier:)`.

### Which annotation reaches golem

| Annotation | Android | iOS |
|------------|---------|-----|
| Compose `contentDescription` | `accessibility_label`, on its own node. The visible text stays a separate text node. | Merged into the element's text. A button with text `+` and description `Increment` reads `"Increment, +"`. On a `Text`, the description **replaces** the visible text. |
| Compose `Modifier.testTag` | `resource-id`, only with `testTagsAsResourceId = true` on an ancestor. golem does not read `resource-id` today. | `accessibility_label`, with no opt-in. |
| Flutter `Semantics(identifier:)` | `resource-id`. golem does not read `resource-id` today. | `accessibility_label`. |
| Visible text | `text` | `text` |

Visible text is the one path that works the same on both platforms. Use it, as
[above](#prefer-visible-text-use-accessibility_label-sparingly). Do not put a
`contentDescription` on a `Text` whose value a step checks, because on iOS the
step then reads the description.

### Element types

`element_type`, and the `button` trait that reads it, are not reliable on a
canvas UI:

- **Android (Compose):** a `Text` is a `TextView`, and every other node is a
  `View`. A Material `Button` adds a separate `Button` node. That node has no text
  or label and is not clickable. The clickable node is the parent `View`, and the
  button's text is a child `TextView`.
- **iOS (Compose Multiplatform):** nodes get real types (`button`, `text`,
  `other`). But the `button` node's text is the merged string, for example
  `"Increment, +"`, and the visible `+` is a child `text` node.

As a result, `{ text = "+", traits = ["button"] }` matches nothing on either
platform. Select by text alone: `{ action = "tap", on_text = "+" }`.

### Bounds and visibility

Every Compose node carries `bounds` and `visible_bounds` on both platforms.
golem's viewport filter drops off-screen nodes, and `scroll` brings them into
view. There are two differences between the platforms:

- **Android** leaves off-screen Compose nodes out of the tree.
- **iOS** keeps every node of a non-lazy layout (for example a `Column` with
  `verticalScroll`) in the tree, with `visible_bounds` equal to `bounds`. The
  bounds are not clipped to the scroll container. A node that is on the screen
  but under a bar outside the container still counts as visible.

`assert_not_visible` searches the full tree, not the visible tree. Thus on iOS it
treats an off-screen node of a non-lazy layout as present, and it waits until
its timeout. A lazy layout (`LazyColumn`) disposes of off-screen items, so
`assert_not_visible` works there on both platforms.

### Merged semantics

`Modifier.semantics(mergeDescendants = true)`, `Modifier.clickable` and Material
components merge their children into one accessibility node. Flutter's
`MergeSemantics` does the same. The merged container looks different on each
platform:

| | Android | iOS |
|---|---------|-----|
| Merged container | No text | Text is the children's text, joined: `"Click A, Click B"` |
| Child text nodes | Present | Present |

Child text nodes stay in the tree on both platforms. Thus these selectors work
on both:

- `on_text` on a child: `{ action = "tap", on_text = "Click B" }` taps inside the
  clickable container.
- `below` / `above` / `left_of` / `right_of` anchored on a child's text.

`inside` and `contains` anchored on the **container's** text work only on iOS,
because the Android container has no text. Anchor on a child's text, or on a
heading, instead. golem has no parent/child selector (there is no `child_of`).
`inside` and `contains` are geometric, so a coarse tree does not change them.

### Flutter

Flutter creates semantics for its standard widgets (`Text`, Material buttons,
text fields). A custom widget, for example a `GestureDetector` on a `Container`
or a `CustomPaint`, exposes nothing until you wrap it in `Semantics`. Thus
semantics are opt-in for each custom widget. This is a bigger task than on
Compose, where most interactive modifiers add semantics.
