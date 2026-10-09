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
