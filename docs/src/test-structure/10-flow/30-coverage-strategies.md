### Coverage Strategies

`coverage` controls how multi-valued `[[flow.apps.devices]]` axes expand into FlowRuns.

| Strategy | Behaviour |
|---|---|
| `smart` (default) | Plan-time set-cover picks fully-pinned slots; a shared coverage group lets the scheduler stop dispatching members once every axis value has been ticked (including bonus ticks — an iPad v26 ticks both the `tablet` and `ios:26` boxes). |
| `min` | Plan-time greedy set-cover — fewest devices that tick every axis value. Every emitted FlowRun runs; no early-stop. |
| `full` | Cartesian product — one FlowRun per (os × type × …) combination. Use when every combo needs independent validation. |
| `one` | Same machinery as `smart` with `max_runs = 1`: first successful run ends the group. Local smoke testing. Tolerates underspec (`ios:latest:2` with only one version available). |

**Two ways to write device constraints**, with different meanings:

*Multi-block form — pinned tuples.* Each `[[flow.apps.devices]]` is an independent combination that must run.

```toml
[[flow.apps.devices]]
os = "ios:26"
type = "tablet"

[[flow.apps.devices]]
os = "android:34"
type = "phone"
```

This guarantees **both specific combinations**: an iPad v26 AND an Android phone v34.

*Single-block array form — independent axes.* Each axis value is a coverage point; Golem ticks every value but doesn't care how the combos fall out.

```toml
[[flow.apps.devices]]
os = ["ios:26", "android:34"]
type = ["tablet", "phone"]
```

This guarantees **every axis value runs somewhere**. Under `smart`/`min` two devices cover all four boxes — could be iPad v26 + Android phone v34, or iPhone v26 + Android tablet v34. Under `full` it emits four fully-pinned combinations.

**When the forms are equivalent.** If each block has at most one multi-valued axis (typically when `type` is absent or single-valued and identical across all blocks), the two forms produce the same boxes:

```toml
# Multi-block
[[flow.apps.devices]]
os = "ios:latest"
type = "phone"
[[flow.apps.devices]]
os = "android:latest"
type = "phone"

# Array form (equivalent — recommended for compactness)
[[flow.apps.devices]]
os = ["ios:latest", "android:latest"]
type = "phone"
```

Both emit two fully-pinned boxes `{ios, latest, phone}` + `{android, latest, phone}` under every strategy. Prefer the array form when it captures the same intent.

**No `[[flow.apps.devices]]` block at all.** Golem runs on whatever platform is currently booted (both if both are booted), on a device of either shape — a simulator is preferred when one is free. Fails fast if nothing is booted.

#### Hardware Axis (virtual / real)

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

Omitting `hardware` means **either shape is acceptable**, and golem prefers a virtual one: a free simulator/emulator always wins over a connected phone, even a phone the app is already installed on. A physical device is used only when no virtual device is free *and already booted* — so a plugged-in phone is usable without editing the flow, without becoming the default target.

That preference lives in the picker, not the constraint, so it applies only where both are already running. It does not reorder the wider ladder: a free connected phone is still chosen ahead of booting a cold simulator, because booting one costs far more than the difference between the two devices.

Say `hardware = "virtual"` when the flow genuinely cannot run on hardware — `push_notification` is the standard case, and it has a lint that says so.

Under `coverage = "one"` / `"smart"`, `hardware = ["virtual", "real"]` gives graceful degradation: the sim box usually succeeds first, the physical box is skipped via the coverage gate. If you want to *insist* on physical, use `hardware = "real"` on its own.

`hardware = "real"` + `create_if_missing = true` errors out — physical hardware cannot be auto-created.

#### Pinning a Specific Device by Name

```toml
[[flow.apps.devices]]
name = "iPhone 15"
```

`name` pins an exact device display name (as shown by `golem devices` / `xcrun simctl list` / `adb devices -l`). Use this when you have a customised simulator or a specific physical device the flow must target.

Under `create_if_missing = true`, a slot with `name = ...` that doesn't match any connected/booted device errors with an actionable message instead of auto-creating a mis-named sim — `name` is a user assertion that the device already exists; golem won't guess its configuration.

#### Auto-Boot Behaviour

When a slot's requirement matches a device that is **shutdown** (no booted match, but a compatible AVD/sim exists), golem boots it automatically and waits for it to be fully ready before continuing. The readiness gate is per-platform:

- **iOS**: `xcrun simctl boot` then `xcrun simctl bootstatus -b` blocks until the sim reports `Booted` with system services up. Typical: 10-25s for a cold boot.
- **Android**: `emulator -avd <id> -no-window -no-audio` spawned detached, then `adb wait-for-device` + poll `getprop sys.boot_completed` until `"1"`. Typical: 60-120s for a cold boot.

**Android emulators always run headless** (`-no-window -no-audio` is hardcoded). Even if you have Android Studio's emulator UI open separately, golem-booted emulators have no GUI window. Useful for CI; if you want to *see* the emulator during local debugging, boot it manually via Android Studio first — golem will reuse the booted device instead of starting another headless one.

iOS sims are headless from `simctl boot` by default, but if you have `Simulator.app` open, it'll attach automatically and show the booted sim. So iOS gives you visibility for free when you want it; Android requires you to boot externally.
