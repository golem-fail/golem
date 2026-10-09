## Nesting and chaining

**Nested anchors** — a relational/containment anchor can itself be a full
selector group, not just bare text. The anchor is its group's first match:

```toml
{ action = "tap", on = { text = "Left", below = { text = "Nested Layout", traits = ["has_text"] } } }
```

**Chaining predicates** — every key in a group is combined with AND. The full
resolution order is:

1. **Match** own-criteria (`text`/`accessibility_label`/state/`traits`) across the
   visible tree, in tree pre-order.
2. **Filter** by each relational/containment predicate (set intersection):
   directional = half-plane **and** cross-axis overlap; containment = full
   enclosure.
3. **Sort** the survivors: containment tightest-first → proximity nearest-first
   (primary-axis gap) → **tree pre-order** as a deterministic tie-break.
4. Apply `index` (0-based) and take the result.

Genuine ties (e.g. a row of equal-distance icons under a full-width heading)
resolve by pre-order — golem does **not** guess; disambiguate with `index` or an
extra predicate.
