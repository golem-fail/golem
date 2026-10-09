## Geometric containment: `contains` / `inside`

Select by spatial nesting — coordinate-based, *not* DOM structure (golem
deliberately does not expose parent/child tree queries; a human perceives
positions, not the document tree).

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

`contains` excludes the anchor itself (an element trivially contains itself) and
coincident zero-margin wrappers, and resolves **smallest-enclosing first**.
