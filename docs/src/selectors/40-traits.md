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
