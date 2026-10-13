## Variables
<!-- summary: Set by --var, [flow.vars], rows, save_to or fixtures; read as ${name}; built-ins such as _device, _os -->

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

### Built-in variables

golem reserves the `_` prefix and fills these in per step:

| Variable | Value |
|---|---|
| `_device` | Device name (`Pixel 9`) |
| `_os` | OS major version (`34`) |
| `_platform` | `android` / `ios` |
| `_type` | Device type (`phone`, `tablet`) |
| `_udid` | Device UDID |
| `_app` | App registry name of the step's app |
| `_hardware` | `virtual` on a sim/emulator, `real` on a physical device — same vocabulary as the `hardware` device constraint |
| `_loop` | 0-based count of times the current block has been entered |
| `_perf` | Last perf snapshot (object — see [Performance Monitoring](#performance-monitoring)) |

`_hardware`, `_loop` and `_perf` are also readable by a branch condition
(`[[block.branch]] if_var = "_hardware", equals = "real"`); the device and app
builtins resolve in `${…}` interpolation only.
