### Selectors

Find the element a step acts on. Prefer `on_text`; use `on_accessibility_label` only to test the a11y label.

| Selector | Description |
|----------|-------------|
| `on_text` | Match by visible text (glob, case-insensitive). **Preferred.** |
| `on_accessibility_label` | Match the accessibility label or the identifier (glob). |
| `on_below` / `on_above` / `on_right_of` / `on_left_of` | Position relative to an anchor element |

```toml
{ action = "tap", on_text = "Submit", on_below = "Counter" }
```

**See [Selectors](selectors.md)** for every selector, state filter and trait, the grouped `on = { … }` form, containment, and how a match is resolved.
