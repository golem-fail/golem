### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

iOS: simulator only; gate on `_hardware`. Android works on emulators and physical devices.

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
