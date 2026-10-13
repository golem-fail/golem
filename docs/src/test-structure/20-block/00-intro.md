## Block

Blocks group steps into logical sections. They execute in document order by default.

```toml
[[block]]
name = "setup"
steps = [
  { action = "assert_visible", on_text = "Welcome", timeout = 30000 },
]

[[block]]
name = "main_test"
steps = [
  { action = "tap", on_text = "+" },
  { action = "tap", on_text = "+" },
  { action = "assert_visible", on_text = "2", on_below = "Counter" },
]
```

`record = true` / `false` on a block overrides the flow's `record` default for that block (`--no-record` still wins).

`max_iterations = N` on a block fails the flow with `EF509` when the flow would run the block an (N+1)th time. It guards a block that loops through `next`, a branch `goto` or `[[data]]` rows: each row counts as one run. To leave a loop on purpose instead, branch on `_loop` (see [Branching](#branching)).
