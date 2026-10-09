<!-- Generated from docs/src/selectors/ — edit the parts there, then run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs`. -->
# Selectors

*How golem finds the element a step acts on.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) · [Actions Reference](actions-reference.md) · [Architecture: visibility model](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)

A selector describes *which* on-screen element a step targets. golem resolves it
against the **visible tree** — the elements a human can actually see (clipped to
ancestor containers).
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

**Grouped** (`on = { … }`, also `to = { … }` / `within = { … }`), required for
`traits`, `contains`, `inside`, and nested anchors:

```toml
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }
```

## Core selectors

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_text` | `text` | Visible text. Glob (`*`, `?`), case-insensitive, anchored (full-string — use globs for partial: `"Item *"`, `"*@*"`). |
| `on_accessibility_label` | `accessibility_label` | The element's accessibility label (aria-label, Android `contentDescription`) or its identifier (iOS `accessibilityIdentifier`, Android `resource-id`). `golem tree` shows these as `label=` and `id=`. **See the guidance below — prefer `text`.** |
| `on_index` | `index` | The Nth match (0-based) after all other filters. |

**What counts as text.** `text` is the text the platform reports as visible.
When an element has no text of its own, its accessibility label is its text,
on both platforms. Thus an icon-only button labelled `Close` matches
`on_text = "Close"`. An image (Android `ImageView`, iOS `image`) is the
exception: its label describes a picture, so it is never text. In a webview,
`text` is the DOM text only.

### Prefer visible `text`; use `accessibility_label` sparingly

Default to `on_text` (or a positional/`contains` selector). Use
`on_accessibility_label` only:

1. **When the test is about the accessibility label itself** — e.g.
   validating screen-reader semantics / a11y compliance. Here the label *is* the
   thing under test.
2. **To navigate to the screen under test, when the tapped element is not what
   the step checks** — e.g. opening a menu by its stable `menu-toggle` id.

Outside those cases, an `accessibility_label` selector tests something the user
never perceives, and silently passes even if the visible text is wrong. When in
doubt, use `text`.

## State filters

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_enabled` | `enabled` | Enabled state (`true`/`false`). |
| `on_checked` | `checked` | Checked/selected state (`true`/`false`). |
| `on_clickable` | `clickable` | Clickable state (`true`/`false`). |

## Traits

Computed predicates on an element's geometry and content. All listed traits in a
selector must hold (AND).

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
  Any positive overlap counts.
- **Nearest-first.** Among matches, the one closest to the anchor (by gap along
  the relation's axis) comes first.

The anchor must be **on-screen**. If it exists but is scrolled off, the
relational match is treated as unresolved (empty).

## Geometric containment: `contains` / `inside`

Select by spatial nesting — coordinate-based, *not* DOM structure. There is no
parent/child selector.

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

`contains` excludes the anchor itself and a wrapper with exactly the anchor's
bounds, and resolves **smallest-enclosing first**.

### `min_matches` — the container of *repeated* items

The smallest box enclosing a *single* item is often a per-item wrapper (a
`<li>`, a list cell), not the list one level up. To target the
**container of several repeated items**, give the `contains` group form a
`min_matches`:

```toml
# the smallest element that encloses ≥2 "Row *" matches — the list, not one row's wrapper
{ action = "assert_visible", on = { contains = { text = "Row *", min_matches = 2 } } }
```

Semantics: *the smallest visible element whose bounds enclose ≥ `min_matches`
elements matching the anchor.* It counts only **visible** matches
(off-screen items are filtered), so the result is the list's on-screen
box. `min_matches` defaults to `1` and must be `1`–`100` (a larger value is
rejected at parse time). `min_matches` is valid **only** on `contains`. To scope
a scroll to a list this way, see [`within`](#within-scoping-a-scroll).

> If a list is so short that only one item is visible, `min_matches = 2` can't
> resolve it — but neither could a human see it's a scrollable list. Make the
> list taller, or fall back to `within = { below = "<heading>" }`.

## Nesting and chaining

**Nested anchors** — a relational/containment anchor can itself be a full
selector group, not just bare text. The anchor is its group's first match:

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
extra predicate.

## Occlusion-aware tapping

The visible tree tells golem what's *clipped or off-screen*, but not what's *covered*
by something painted on top (a sticky header, a `z-index` overlay). So golem
**hit-tests** the target before tapping and **routes around** an occluder: a plain
`tap` lands on the first clear sample point, so a button
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
separate, system-level concern.

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

See [`min_matches`](#min_matches--the-container-of-repeated-items) and
[Actions Reference → scroll](actions-reference.md) for the full action.

## Canvas-rendered UI (Compose, Compose Multiplatform, Flutter)

Jetpack Compose, Compose Multiplatform and Flutter draw the whole UI into one
platform view. Each framework copies its semantics tree into the platform
accessibility tree, so golem sees one node for each widget. Text selectors,
relational selectors and viewport filtering work on these nodes.

What changes is *which* annotation reaches golem, and how coarse the tree is.
The facts below were checked with Jetpack Compose, Compose Multiplatform 1.11
and Flutter 3.47, on Android API 36 and iOS 26.

### Version floors

- **Compose Multiplatform 1.8.0 or later on iOS.** From 1.8.0 the Compose
  accessibility tree syncs to iOS automatically.
- **Flutter 3.19 or later** for `Semantics(identifier:)`.

### Which annotation reaches golem

| Annotation | Android | iOS |
|------------|---------|-----|
| Compose `contentDescription` | `accessibility_label`, on its own node, which has no other text, so the description is also that node's `text`. The visible text stays a separate text node. | Merged into the element's text. A button with text `+` and description `Increment` reads `"Increment, +"`. On a `Text`, the description **replaces** the visible text. |
| Compose `Modifier.testTag` | Identifier (`resource-id`), only with `testTagsAsResourceId = true` on an ancestor. | Identifier (`accessibilityIdentifier`), with no opt-in. |
| Flutter `Semantics(identifier:)` | Identifier (`resource-id`). | Identifier (`accessibilityIdentifier`). |
| Flutter widget text (`Text`, button text) | The label; golem reads it as `text` (see [What counts as text](#core-selectors)). | `text` |
| Visible text | `text` | `text` |

The `accessibility_label` selector matches the label or the identifier.

Visible text is the one path that works the same on both platforms. Use it (see
[Prefer visible `text`](#prefer-visible-text-use-accessibility_label-sparingly)). Do not put a
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
- **Flutter:** a button is a `Button` (Android) or `button` (iOS) node that
  holds its own text. A `Text` is a plain `View` on Android and `text` on iOS.
  An `Icon` with a `semanticLabel` looks the same as a `Text` on both
  platforms. Only `Semantics(image: true)` and `Image` give an image node
  (`ImageView` / `image`).

As a result, on Compose `{ text = "+", traits = ["button"] }` matches nothing on
either platform. Select by text alone: `{ action = "tap", on_text = "+" }`.

### Bounds and visibility

Every Compose and Flutter node carries `bounds` and `visible_bounds` on both
platforms. golem's viewport filter drops off-screen nodes, and `scroll` brings
them into view. Compose has two differences between the platforms:

- **Android** leaves off-screen Compose nodes out of the tree.
- **iOS** keeps every node of a non-lazy layout (for example a `Column` with
  `verticalScroll`) in the tree, with `visible_bounds` equal to `bounds`. That
  is, Compose Multiplatform does not clip `visible_bounds` to the scroll
  container.

A lazy Flutter `ListView` leaves its off-screen items out of the Android tree.
On iOS it keeps them, with a zero-size frame at the origin.

Both kinds of node fall outside the viewport, so `assert_not_visible` treats
them as absent on both platforms, as a user would. One caveat remains on iOS:
a non-lazy Compose node that its scroll container hides but that is still
inside the screen keeps its full `visible_bounds`, so golem counts it as on
screen.

### Merged semantics

`Modifier.semantics(mergeDescendants = true)`, `Modifier.clickable` and Material
components merge their children into one accessibility node. Flutter's
`MergeSemantics` does the same, but the two frameworks report the result
differently:

| | Compose, Android | Compose, iOS | Flutter, both platforms |
|---|---|---|---|
| Merged container | No text | Text is the children's text, joined: `"Click A, Click B"` | Text is the children's text, joined by a line break: `"Merged A\nMerged B"` |
| Child text nodes | Present | Present | **Absent** |

On Compose, child text nodes stay in the tree on both platforms. Thus these
selectors work on both:

- `on_text` on a child: `{ action = "tap", on_text = "Click B" }` taps inside the
  clickable container.
- `below` / `above` / `left_of` / `right_of` anchored on a child's text.

`inside` and `contains` anchored on the **container's** text work only on iOS,
because the Android container has no text. Anchor on a child's text, or on a
heading, instead.

On Flutter, a merged child has no node of its own. Select the container by its
joined text, with a glob: `{ action = "tap", on_text = "Merged A*" }`. `inside`
and `contains` are geometric, so a coarse tree does not change them.

### Flutter

A Flutter custom widget (for example a `GestureDetector` on a `Container`, or a
`CustomPaint`) is invisible to golem until you wrap it in `Semantics`.
