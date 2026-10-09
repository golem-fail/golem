### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

**Simulator-only on iOS; Android works everywhere.** The iOS path clears the app's data container through a host filesystem path that `simctl` hands back, which only exists for a simulator — on a physical device the container lives on the device and `get_app_container` returns a path the host can't reach. Android uses `adb shell pm clear`, which is device-agnostic. On a physical iPhone the driver bails pointing at this paragraph.

To reset state on a physical iOS device, either drive the app's own "sign out" / "reset" affordance, or reinstall it (the install script runs before every flow; `GOLEM_REBUILD` forces a fresh build). Gate the step on device class if one flow must cover both:

```toml
[[block.branch]]
if_var = "_hardware"
equals = "virtual"
goto = "wipe_via_clear_data"
[[block.branch]]
if_var = "_hardware"
equals = "real"
goto = "wipe_via_app_ui"
```
