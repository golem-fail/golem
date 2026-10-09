### Block `next`

Jump to a named block after completion (instead of falling through). A block with `[[block.branch]]` entries ignores `next`.

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
