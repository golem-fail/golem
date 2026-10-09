### `set_dark_mode` — Set dark mode

Switch the device's system appearance to dark (`enabled = true`) or light (`enabled = false`).

```toml
{ action = "set_dark_mode", enabled = true }
{ action = "set_dark_mode", enabled = false }
```

iOS: simulator only; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch).
