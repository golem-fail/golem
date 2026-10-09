## Data-Driven Tests

A `[[data]]` table holds the rows, and a block iterates them with
`for_each = "data"`. The block runs once per row, and each row's fields are
read under the `${_each.<field>}` prefix:

```toml
[[data]]
user = "alice"

[[data]]
user = "bob"

[[block]]
for_each = "data"
steps = [
  { action = "type", on_text = "Search", input = "${_each.user}" },
  { action = "assert_visible", on_text = "${_each.user}" },
]
```

Only the `for_each` block repeats — surrounding blocks run once, and the
repeating block re-enters per row (`block:0`, `block:1`, … in step labels and
recordings). An empty `[[data]]` table runs the block zero times.

Iteration is **block-level only**: rows parameterise steps inside a flow, not
whole flows. The block re-enters without relaunching the app, so a row that
leaves the app somewhere new is the next row's starting state — put anything
that must be reset into the block's own steps. To run a whole scenario per
case — each with a fresh app launch and its own pass/fail line — write it as
its own flow, or as a subflow invoked with different `[block.vars]`.
