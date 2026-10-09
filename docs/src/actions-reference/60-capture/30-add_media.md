### `add_media` — Push media to device

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

**Simulator-only on iOS** — `simctl addmedia` can't address a physical device, so the driver refuses there; put the fixture in the device's library ahead of the run, or gate the step on `_hardware`. Android uses `adb push` plus a media-scanner broadcast and works on physical devices and emulators alike. See [Device Controls](#device-controls) for the other two simulator-backed actions.
