## Subflow

Delegate a block to a child flow file. The child inherits parent variables and device context.

```toml
# parent.test.toml
[[block]]
name = "increment"
run_flow = "subflows/increment_counter.test.toml"

[block.save_to]
counter_value = "result_after_increment"
```

```toml
# subflows/increment_counter.test.toml
[flow]
name = "Increment counter"
explicit_only = true        # Skip in the bulk sweep (see below)

[flow.options]
app_lifecycle = "manual"    # Don't restart the app

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "read", on_below = "Counter", on_index = 0, save_to = "counter_value" },
]
```

Variables listed in `[block.save_to]` propagate back to the parent. Override child variables with `[block.vars]`.

A subflow is a normal flow, so `golem run` (no path) would otherwise discover and run it standalone — redundant with the flows that embed it. Mark it `explicit_only = true` in `[flow]` to keep it out of the **bulk sweep** while still running it when you target it:

| Invocation | `explicit_only` flow |
|---|---|
| `golem run` (no path) | **skipped** — the bulk sweep |
| `golem run <dir>` (no `--tag`) | **skipped** |
| `golem run --tag login` (tag matches) | **runs** — a matching tag opts it in |
| `golem run --tag other` (no matching tag) | skipped |
| `golem run path/to/sub.test.toml` | **runs** — path given directly |
| `golem run 'e2e/**/*.test.toml'` (shell glob) | **runs** — the shell expands the glob to file paths before golem sees it, so a globbed path is indistinguishable from a typed one |

In short: `explicit_only` suppresses only the tag-less discovery sweep. Tag it to include it in specific `--tag` runs; name its path to run it directly. Set `app_lifecycle = "manual"` so the child inherits the parent's already-launched app (see [Lifecycle](#lifecycle-setup--teardown)).
