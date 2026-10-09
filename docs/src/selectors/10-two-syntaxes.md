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
