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
