### Selectors

Find elements by visible text, position, containment, traits, or state. Common
selectors:

| Selector | Description |
|----------|-------------|
| `on_text` | Match by visible text (glob, case-insensitive). **Preferred.** |
| `on_accessibility_label` | Match by accessibility id. Use only when *testing* the a11y label (screen readers) or as a throwaway shortcut to navigate — prefer `on_text` otherwise. |
| `on_index` | Match the Nth element (0-based) |
| `on_enabled` / `on_checked` / `on_clickable` | Filter by state |
| `on_below` / `on_above` / `on_right_of` / `on_left_of` | Position relative to an anchor (column/row-aware) |

Use the grouped form (`on = { … }`) for `traits`, geometric `contains`/`inside`,
and nested anchors:

```toml
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }
{ action = "assert_visible", on = { contains = { text = "Item 0" } } }
```

**See [Selectors](selectors.md)** for the full reference: every selector and
trait, the column/row-overlap and nearest-first relational rules, `contains`/
`inside`, nesting/chaining, and the match→filter→sort resolution order.
