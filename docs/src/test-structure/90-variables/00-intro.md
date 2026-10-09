## Variables

Set variables with `--var NAME=value` on the CLI, in `[flow.vars]`, from data rows, with a step's `save_to`, or from fixtures. Reference them as `${name}`:

```toml
[flow.vars]
base_url = "https://staging.example.com"

[[block]]
steps = [
  { action = "read", on_right_of = "Status:", save_to = "current_status" },
  { action = "bash", run = "echo ${current_status}", save_to = "result" },
]
```

When the CLI, `golem.toml` and the flow set the same name, see [Project config](#project-config-golemtoml) for which one wins.
