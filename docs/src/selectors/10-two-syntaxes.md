## Two syntaxes
<!-- summary: Flat on_* keys, or grouped on = { … } for traits, contains, inside -->

**Flat** (`on_*` fields) for simple cases:

```toml
{ action = "tap", on_text = "Submit", on_below = "Counter" }
```

**Grouped** (`on = { … }`, also `to = { … }` / `within = { … }`), required for
`traits`, `contains`, `inside`, and nested anchors:

```toml
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }
```
