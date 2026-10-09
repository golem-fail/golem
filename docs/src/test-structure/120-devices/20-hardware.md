### Hardware Axis (virtual / real)

```toml
[[flow.apps.devices]]
# (hardware omitted)                # default: either shape, virtual preferred

[[flow.apps.devices]]
hardware = "virtual"                # explicit: virtual-only

[[flow.apps.devices]]
hardware = "real"                   # physical device required

[[flow.apps.devices]]
hardware = ["virtual", "real"]      # coverage axis — both tick boxes emitted
```

Omitting `hardware` means **either shape is acceptable**, and golem prefers a virtual one: a free booted simulator/emulator wins over a connected phone. A physical device is used only when no virtual device is free and already booted, so a plugged-in phone is usable without editing the flow.

Say `hardware = "virtual"` when the flow cannot run on hardware — `push_notification` is the standard case, and it has a lint that says so.

Under `coverage = "one"` / `"smart"`, `hardware = ["virtual", "real"]` degrades gracefully: the virtual box usually succeeds first, and the physical box is skipped. To *insist* on physical, use `hardware = "real"` on its own.

`create_if_missing` is a `[flow.options]` (or `golem.toml` `[options]`) key, default `false`. When no booted or shut-down device matches a slot, `true` makes golem create and boot a simulator/emulator from the slot's `os` and `type` (a phone if `type` is unset); `false` fails the slot with "No … devices found". It needs an `os` that names a platform. When no matching real device is connected, `hardware = "real"` with `create_if_missing = true` errors: golem cannot create physical hardware.
