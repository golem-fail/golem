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
