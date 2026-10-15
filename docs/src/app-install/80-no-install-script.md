## When no `install_script` is configured
<!-- summary: Without install_script, golem launches the installed app -->

If an app has no `install_script` field in `golem.toml` or its flow file, golem behaves like a permanent `--no-build` for that app: no install runs, the flow goes straight to `launch`. The two paths converge on the success case but differ on the failure mode:

| Scenario | App present | App absent |
|---|---|---|
| **No `install_script` in TOML** | launches, runs flow | `launch` errors at runtime, cache marks `FailedNoScript`, subsequent flows skip with that reason |
| **`--no-build` flag** | mark `Succeeded` upfront, runs flow | flow fails immediately with an actionable hint to drop the flag |

So if your project has no install scripts anywhere, `--no-build` is redundant — golem already assumes the apps are preinstalled. The flag earns its keep when scripts *are* configured and you want to bypass them temporarily.
