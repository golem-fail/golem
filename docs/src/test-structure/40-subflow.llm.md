## Subflow

A block with `run_flow` runs another flow file instead of its own steps. The child sees the parent's variables and runs on the same device.

```toml
# parent.test.toml
[[block]]
name = "increment"
run_flow = "subflows/increment_counter.test.toml"   # path relative to this flow

[block.vars]                         # set in the child
step = "1"

[block.save_to]                      # child variable = parent variable to write
counter_value = "result_after_increment"
```

```toml
# subflows/increment_counter.test.toml
[flow]
name = "Increment counter"
explicit_only = true

[flow.options]
app_lifecycle = "manual"

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "read", on_below = "Counter", save_to = "counter_value" },
]
```

- If the child fails, the parent flow fails. A `[block.save_to]` key the child never set fails the flow.
- `explicit_only` skips tag-less discovery (`golem run`, `golem run <dir>`); a matching `--tag` or a direct path still runs it.
- Set `app_lifecycle = "manual"` in the child so it keeps the parent's running app.
