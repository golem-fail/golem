# Test Structure

*Anatomy of a flow.*

← [Back to README](../README.md)

Tests are written in TOML. A `.test.toml` file defines a **flow** — the top-level unit of execution.

## Contents

- [Flow](#flow) — [Options](#flow-options), [Coverage strategies](#coverage-strategies), [Performance Monitoring](#performance-monitoring)
- [Block](#block) — [Platform-specific](#platform-specific-blocks), [Branching](#branching), [`next`](#block-next)
- [Step](#step) — [Selectors](#selectors), [Grouped syntax](#grouped-selector-syntax), [Options](#step-options), [Timeout multipliers](#timeout-multipliers)
- [Subflow](#subflow)
- [Teardown](#teardown)
- [Data-Driven Tests](#data-driven-tests)
- [Variables](#variables)
- [Fake Data Generators](#fake-data-generators)
- [Multi-App Flows](#multi-app-flows)

See also: [Actions Reference](actions-reference.md) for every action and its params.

## Flow

A flow is a complete test scenario: metadata, app configuration, device targets, execution blocks, and optional teardown.

```toml
[flow]
name = "Login test"
tags = ["auth", "smoke"]
# start = "block_name"  # Optional: skip to this block (assumes app in correct state)

[[flow.apps]]
name = "app"
bundle = "com.example.myapp"

[[flow.apps.devices]]
os = "ios:latest"
type = "phone"

[[flow.apps.devices]]
os = "android:latest"
type = "phone"

[[block]]
name = "login"
steps = [
  { action = "type", on_text = "Email", input = "user@example.com" },
  { action = "type", on_text = "Password", input = "secret" },
  { action = "tap", on_text = "Sign In" },
  { action = "assert_visible", on_text = "Dashboard", timeout = 10000 },
]
```

The flow runs on every device listed. Golem launches the first app automatically before executing blocks.

The `name` you give an app here is the **canonical reference** for it from any action with an `app` field (`launch`, `stop`, `push_notification`, `grant_permission`, etc.). Use the name — `app = "app"` — rather than the bundle id. Bundle ids are accepted as a fallback but they're an implementation detail; the name keeps flows readable and survives bundle-id renames. The [Multi-App Flows](#multi-app-flows) section shows the pattern.

### Flow Options

```toml
[flow.options]
step_timeout = 5000                 # Base timeout (ms), default: 5000. See timeout multipliers below.
max_steps = 10000                   # Safety limit
max_runtime = "30m"                 # "5m", "2h", "500ms"
app_lifecycle = "reset"             # "reset" (default), "launch", "manual"
screenshot_on_failure = true        # Auto-capture screenshot on step failure (default: true)
record = true                       # Default every block to record (block can opt out with `record = false`)
coverage = "smart"                  # "smart" (default), "min", "full", "one" — see Coverage strategies
perf = true                         # Performance monitoring (default: true)
perf_memory_warn_mb = 200.0
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0
perf_cpu_error_percent = 95.0
a11y = "relaxed"                    # Accessibility audit: "off", "critical", "relaxed" (default), "strict". --a11y overrides.
a11y_max_errors = 0                 # Optional: fail the flow if cumulative a11y errors exceed this
a11y_max_warnings = 20              # Optional: fail the flow if cumulative a11y warnings exceed this
a11y_min_confidence = 0.8           # Optional: drop findings below this confidence (0–1). Deterministic checks are 1.0.
```

### Accessibility Audit

After each block, Golem audits the **visible** UI tree for accessibility issues
(zero config, on by default at `relaxed`). Findings appear inline in the live
run (a per-block `a11y: N error(s), M warning(s)` line; `--verbose` lists each)
and in every report format (`json`/`junit`/`toon`/`human`). They are warnings by
default; set `a11y_max_errors`/`a11y_max_warnings` to fail a flow. Levels: `off`,
`critical` (tree checks only), `relaxed` (default), `strict` (forces a per-block
screenshot for the contrast check + WCAG-AAA warn bands).

Tree checks judge only the innermost actionable control (the real tap target),
and size thresholds are dp-normalised (so verdicts match across Android px and
iOS points). Each finding carries a **confidence** (0–1): deterministic checks
are `1.0`; the heuristic pixel check (contrast) scores lower when the region is
busy or the ratio is borderline. `a11y_min_confidence` filters out findings
below a threshold so flows can tune heuristic noise.

| Check | Severity | Screenshot? | Rule |
|-------|----------|-------------|------|
| `missing_label` | Error | no | Actionable control with no text/accessibility label anywhere in its subtree |
| `touch_target_too_small` | Error/Warning | no | Min dimension below the dp threshold (error `<24dp`, warn `<44dp`; `strict` errors `<44dp`) |
| `text_too_small` | Warning | no | Text element whose **box** is shorter than the min dp height (10dp; `strict` 12dp) — certain, since glyphs can't exceed their box |
| `duplicate_labels` | Warning | no | Sibling controls sharing identical visible text |
| `overlapping_interactive` | Warning | no | Sibling controls with overlapping bounds (coincident/enclosed wrappers excluded) |
| `low_contrast` | Error/Warning | **strict** | Text/background WCAG contrast below AA (4.5:1, or 3:1 large); `strict` also warns below AAA. Heuristic — carries a confidence score |

### Coverage strategies

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

**No `[[flow.apps.devices]]` block at all.** Golem runs on whatever platform is currently booted (both if both are booted). Virtual-only (sim/emulator) by default — physical devices are never picked implicitly. Fails fast if nothing is booted.

#### Hardware axis (virtual / real)

```toml
[[flow.apps.devices]]
# (hardware omitted)                # default: virtual-only (sim/emulator)

[[flow.apps.devices]]
hardware = "virtual"                # explicit: virtual-only

[[flow.apps.devices]]
hardware = "real"                   # physical device required

[[flow.apps.devices]]
hardware = ["virtual", "real"]      # coverage axis — both tick boxes emitted
```

Physical devices require **explicit opt-in** via `hardware = "real"`. The default is virtual-only so an accidentally-connected phone doesn't get swept into a flow it wasn't meant for.

Under `coverage = "one"` / `"smart"`, `hardware = ["virtual", "real"]` gives graceful degradation: the sim box usually succeeds first, the physical box is skipped via the coverage gate. If you want to *insist* on physical, use `hardware = "real"` on its own.

`hardware = "real"` + `create_if_missing = true` errors out — physical hardware cannot be auto-created.

#### Pinning a specific device by name

```toml
[[flow.apps.devices]]
name = "iPhone 15"
```

`name` pins an exact device display name (as shown by `golem devices` / `xcrun simctl list` / `adb devices -l`). Use this when you have a customised simulator or a specific physical device the flow must target.

Under `create_if_missing = true`, a slot with `name = ...` that doesn't match any connected/booted device errors with an actionable message instead of auto-creating a mis-named sim — `name` is a user assertion that the device already exists; golem won't guess its configuration.

#### Auto-boot behaviour

When a slot's requirement matches a device that is **shutdown** (no booted match, but a compatible AVD/sim exists), golem boots it automatically and waits for it to be fully ready before continuing. The readiness gate is per-platform:

- **iOS**: `xcrun simctl boot` then `xcrun simctl bootstatus -b` blocks until the sim reports `Booted` with system services up. Typical: 10-25s for a cold boot.
- **Android**: `emulator -avd <id> -no-window -no-audio` spawned detached, then `adb wait-for-device` + poll `getprop sys.boot_completed` until `"1"`. Typical: 60-120s for a cold boot.

**Android emulators always run headless** (`-no-window -no-audio` is hardcoded). Even if you have Android Studio's emulator UI open separately, golem-booted emulators have no GUI window. Useful for CI; if you want to *see* the emulator during local debugging, boot it manually via Android Studio first — golem will reuse the booted device instead of starting another headless one.

iOS sims are headless from `simctl boot` by default, but if you have `Simulator.app` open, it'll attach automatically and show the booted sim. So iOS gives you visibility for free when you want it; Android requires you to boot externally.

### Performance Monitoring

Golem captures app performance metrics after each block (unless `--no-perf` or `perf = false`). Metrics are collected from the device via platform tools and the companion app.

| Metric | Unit | Source |
|--------|------|--------|
| Memory | MB | `dumpsys meminfo` (Android), `footprint_in_bytes` (iOS) |
| CPU | % | `dumpsys cpuinfo` (Android), `cpu_usage` (iOS) |
| Threads | count | `/proc/<pid>/status` (Android), `threadCount` (iOS) |
| File descriptors | count | Companion `/perf` endpoint |
| Disk | MB | Companion `/perf` (Android), `du -sk` (iOS) |
| Network RX/TX | KB | Companion `/perf` (Android), `netstat` (iOS) |

Thresholds in `[flow.options]` trigger warnings or failures:

```toml
perf_memory_warn_mb = 200.0     # Warn if memory exceeds 200 MB
perf_memory_error_mb = 500.0    # Fail if memory exceeds 500 MB
perf_cpu_warn_percent = 80.0    # Warn if CPU exceeds 80%
perf_cpu_error_percent = 95.0   # Fail if CPU exceeds 95%
```

Performance data appears in all output formats: human (table), JSON (objects), JUnit (properties), toon (abbreviated codes).

## Block

Blocks group steps into logical sections. They execute in document order by default.

```toml
[[block]]
name = "setup"
steps = [
  { action = "assert_visible", on_text = "Welcome", timeout = 30000 },
]

[[block]]
name = "main_test"
steps = [
  { action = "tap", on_text = "+" },
  { action = "tap", on_text = "+" },
  { action = "assert_visible", on_text = "2", on_below = "Counter" },
]
```

### Platform-Specific Blocks

Skip blocks that don't apply to the current device:

```toml
[[block]]
name = "android_back"
where = { os = "android" }
steps = [
  { action = "press", button = "back" },
]
```

### Branching

Control flow between blocks with conditions:

```toml
[[block]]
name = "check_state"
steps = [
  { action = "assert_visible", on_text = "Welcome", if_fail = "ignore" },
]

[[block.branch]]
if_visible = "Dashboard"
goto = "already_logged_in"

[[block.branch]]
goto = "login_required"            # Unconditional fallback
```

Branch conditions: `if_visible`, `if_not_visible`, `if_var` + `equals`/`matches`/`gte`.

### Block `next`

Jump to a named block after completion (instead of falling through):

```toml
[[block]]
name = "step_a"
next = "step_c"
steps = [...]

[[block]]
name = "step_b"
steps = [...]    # Skipped

[[block]]
name = "step_c"
steps = [...]    # Executed after step_a
```

## Step

A step is a single action with optional selectors, timeouts, and error handling.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "assert_visible", on_text = "1", on_below = "Counter", timeout = 5000 }
{ action = "type", on_text = "Email", input = "hello@example.com" }
{ action = "read", on_right_of = "Status:", save_to = "status_value" }
```

### Selectors

Find elements by visible text, position, containment, traits, or state. Common
selectors:

| Selector | Description |
|----------|-------------|
| `on_text` | Match by visible text (glob, case-insensitive). **Preferred.** |
| `on_accessibility_label` | Match by accessibility id. Use only when *testing* the a11y label (screen readers) or as a throwaway shortcut to navigate — prefer `on_text` otherwise. |
| `on_index` | Match the Nth element (0-based) |
| `on_enabled` / `on_checked` / `on_clickable` | Filter by state |
| `on_below` / `on_above` / `on_right_of` / `on_left_of` | Position relative to an anchor (column/row-aware) |

Use the grouped form (`on = { … }`) for `traits`, geometric `contains`/`inside`,
and nested anchors:

```toml
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }
{ action = "assert_visible", on = { contains = { text = "Item 0" } } }
```

**See [Selectors](selectors.md)** for the full reference: every selector and
trait, the column/row-overlap and nearest-first relational rules, `contains`/
`inside`, nesting/chaining, and the match→filter→sort resolution order.

### Step Options

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | per-action | Max wait in ms. Overrides computed default. |
| `auto_scroll` | `false` | Scroll page to find element |
| `max_scrolls` | — | Limit scroll attempts |
| `if_fail` | `"error"` | `"error"` (fail flow), `"warn"` (log + continue), `"ignore"` (silent continue) |
| `retry` | `0` | Retry count on failure |
| `retry_delay` | `1000` | Delay between retries (ms) |
| `save_to` | — | Save result to a variable |
| `app` | — | Target a specific app (for multi-app flows) |

### Timeout Multipliers

Each action has a built-in multiplier applied to the base timeout (`step_timeout`, default 5000ms). Per-step `timeout` always overrides. `auto_scroll = true` forces 6x minimum.

| Multiplier | Timeout (at 5s base) | Actions |
|------------|---------------------|---------|
| 1x | 5s | `tap`, `double_tap`, `backspace`, `long_press`, `swipe`, `pinch`, `gesture`, `press`, `rotate`, `screenshot`, `hide_keyboard`, device controls |
| 2x | 10s | `type`, `assert_visible`, `assert_not_visible`, `read`, alerts |
| 3x | 15s | `launch`, `stop` |
| 4x | 20s | `bash`, `run` |
| 6x | 30s | `scroll`, `auto_scroll`, `*_http`, `open_link` |
| 48x | 240s | `await_email` |

Actions with intrinsic duration (`long_press`, `type`, `rotate`, `gesture`) auto-extend: `max(multiplied, duration + 2s)`. For `type`, duration is ~200ms per character.

## Subflow

Delegate a block to a child flow file. The child inherits parent variables and device context.

```toml
# parent.test.toml
[[block]]
name = "increment"
run_flow = "subflows/increment_counter.test.toml"

[block.save_to]
counter_value = "result_after_increment"
```

```toml
# subflows/increment_counter.test.toml
[flow]
name = "Increment counter"

[flow.options]
app_lifecycle = "manual"    # Don't restart the app

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "read", on_below = "Counter", on_index = 0, save_to = "counter_value" },
]
```

Variables listed in `[block.save_to]` propagate back to the parent. Override child variables with `[block.vars]`.

## Teardown

> **Not yet wired.** Teardown blocks are parsed but not executed. This section describes the intended behavior.

Teardown blocks run after the flow completes, regardless of pass/fail. Failures in teardown don't affect the test result.

```toml
[[teardown]]
steps = [
  { action = "screenshot", path = "/tmp/final.png" },
  { action = "stop", app = "app" },
]
```

Skip teardown with `--no-teardown`.

## Data-Driven Tests

Run the entire flow once per data row:

```toml
[[data]]
username = "alice"
password = "pass1"

[[data]]
username = "bob"
password = "pass2"

[[block]]
steps = [
  { action = "type", on_text = "Email", input = "${username}" },
  { action = "type", on_text = "Password", input = "${password}" },
  { action = "tap", on_text = "Login" },
]
```

## Variables

Set variables from the CLI, flow metadata, data rows, `read` actions, or fixtures:

```bash
golem run flows/login.test.toml --var EMAIL=test@example.com
```

```toml
[flow.vars]
base_url = "https://staging.example.com"

[[block]]
steps = [
  { action = "read", on_right_of = "Status:", save_to = "current_status" },
  { action = "bash", run = "echo ${current_status}", save_to = "result" },
]
```

## Fake Data Generators

Generate realistic test data with `${fake:…}` — `email`, `password`, `uuid`,
`number`, `phone`, `city`, and the structured `person` / `address` /
`credit_card`. Values are random but valid; `--seed <N>` replays deterministically.

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
```

`${fake:person}` is country-aware and exposes each name part as a
`given` / `family` pair across scripts — `person.given` / `.family` (primary,
country-aware), `person.reading.*` (furigana/reading), `person.ascii.*` (Latin),
plus raw per-script branches like `person.katakana.*` and `person.hangul.*`.
There is no joined full name — a form decides order and separator.

**See [Fake Data Generators](fake-data.md) for the full reference**: every
simple generator, the structured generators, and the `person` representation /
chain / `country` model.

## Multi-App Flows

Test interactions across multiple apps:

```toml
[[flow.apps]]
name = "app"
bundle = "com.example.main"

[[flow.apps]]
name = "companion"
bundle = "com.example.companion"

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "launch", app = "companion" },
  { action = "assert_visible", on_text = "Shared Data" },
  { action = "launch", app = "app" },
  { action = "stop", app = "companion" },
]
```

`launch` brings an app to foreground without restarting it. Use `restart = true` for a cold start.
