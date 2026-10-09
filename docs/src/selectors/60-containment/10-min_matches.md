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
