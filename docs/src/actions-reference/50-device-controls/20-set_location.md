### `set_location` — Set GPS coordinates

Set the device's GPS position to `latitude` / `longitude` (decimal degrees).

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

iOS: simulator only; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch).
