### Block `next`
<!-- summary: Go to a named block after this one; with a branch, the else -->

Jump to a named block after completion (instead of falling through). On a block with a `branch`, `next` is the else: the flow goes there when no arm matches.

```toml
[[block]]
name = "step_a"
next = "step_c"
steps = [...]

[[block]]
name = "step_b"
steps = [...]    # Skipped

[[block]]
name = "step_c"
steps = [...]    # Executed after step_a
```
