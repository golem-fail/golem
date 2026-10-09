### `set_location` — Set GPS coordinates

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

> **The iOS device controls are simulator-backed.** `set_dark_mode`, `set_location` and `add_media` all drive `simctl`, which only addresses simulators — on a physical iPhone the driver refuses, naming the action and the device, rather than surfacing a raw `simctl` error. Gate the step on `_hardware` if a flow has to run on both shapes. The Android equivalents go through `adb` and work on emulators and physical devices alike.
