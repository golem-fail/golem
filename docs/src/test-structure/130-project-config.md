## Project config (`golem.toml`)

`golem.toml` sits at the project root (golem walks up from the working
directory to find it) and holds the defaults every flow inherits. A flow
always wins over the project for anything it states itself.

```toml
[vars]                      # referenced in any flow as ${base_url}
base_url = "https://staging.example.com"

[options]                   # defaults for every flow's [flow.options]
step_timeout = 8000
record = true
max_device_wait = "30m"     # queue-wait cap; --max-device-wait beats it

[[apps]]                    # app registry — flows reference by name
name = "app"
bundle = "com.example.myapp"
install_script = "scripts/install.sh"

[[teardown]]                # appended to every flow's own teardown
steps = [
  { action = "bash", run = "scripts/cleanup.sh" },
]

[device_settings]           # OS-level tweaks applied once per device session
android = { "secure.long_press_timeout" = "400" }
```

| Section | Merge rule |
|---|---|
| `[vars]` | Flow `[flow.vars]` of the same name wins |
| `[options]` | Per field: a flow's `[flow.options]` value wins, others fall through |
| `[[apps]]` | Flows inherit `bundle` / `install_script` / `install_timeout_ms` / `devices` by app name |
| `[[teardown]]` | Runs after the flow's own teardown, and runs even when the flow failed |
| `[device_settings]` | Applied to the device before any flow runs — not a flow-level concept |

CLI flags beat both: `--var` overrides a project or flow var, and the
recording flags override `record` at every level.
