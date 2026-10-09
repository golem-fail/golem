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
