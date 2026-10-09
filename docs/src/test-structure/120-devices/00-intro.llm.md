## Devices

Most flows need no device settings: with no `[[flow.apps.devices]]`, the flow runs on whatever is booted (both platforms if both are booted). Set constraints only when the flow must run on a specific OS, shape or device.

```toml
[[flow.apps.devices]]
os = ["ios:latest", "android:34"]   # "ios:26", "ios:18.6", "ios:17+" (minimum), "ios:latest", "ios:latest:2"
type = "phone"                      # "phone" | "tablet"
hardware = "virtual"                # "virtual" | "real" | ["virtual", "real"]; omitted = either, virtual preferred
# name = "iPhone 15"                # exact device name
```

- Each `[[flow.apps.devices]]` entry is one combination that must run. An array inside one entry is an axis: every value must run somewhere.
- `[flow.options] coverage`: `smart` (default) ticks every axis value, using more devices when free; `min` uses the fewest devices that tick every value; `full` runs every combination; `one` stops after the first successful run.
- Use `hardware = "virtual"` for flows with `push_notification`.
- `[flow.options] create_if_missing = true` creates a simulator/emulator when none matches. It needs an `os` with a platform, and errors for `hardware = "real"` or `name`.
