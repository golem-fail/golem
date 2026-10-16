<!-- Generated from docs/src/test-structure/ — edit the parts there, then run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs`. -->
# Test Structure

*Anatomy of a flow.*

← [Back to README](../README.md) · See also [Actions Reference](actions-reference.md)

Tests are written in TOML. A `.test.toml` file defines a **flow** — the top-level unit of execution.

## Contents

- [Flow](#flow)
  - [Launch-time permissions](#launch-time-permissions)
  - [Flow Options](#flow-options)
  - [Accessibility Audit](#accessibility-audit)
  - [Performance Monitoring](#performance-monitoring)
- [Block](#block)
  - [Platform-Specific Blocks](#platform-specific-blocks)
  - [Branching](#branching)
  - [Block `next`](#block-next)
- [Step](#step)
  - [Selectors](#selectors)
  - [Step Options](#step-options)
  - [Timeout Multipliers](#timeout-multipliers)
- [Subflow](#subflow)
- [Reuse: Subflow vs Mixin vs Fixture](#reuse-subflow-vs-mixin-vs-fixture)
- [Lifecycle: Setup & Teardown](#lifecycle-setup--teardown)
- [Teardown](#teardown)
- [Data-Driven Tests](#data-driven-tests)
- [Variables](#variables)
  - [Built-in variables](#built-in-variables)
- [Secrets](#secrets)
- [Fake Data Generators](#fake-data-generators)
- [Multi-App Flows](#multi-app-flows)
- [Devices](#devices)
  - [Coverage Strategies](#coverage-strategies)
  - [Hardware Axis (virtual / real)](#hardware-axis-virtual--real)
  - [Pinning a Specific Device by Name](#pinning-a-specific-device-by-name)
  - [Auto-Boot Behaviour](#auto-boot-behaviour)
- [Project config (`golem.toml`)](#project-config-golemtoml)

## Flow
<!-- summary: [flow] fields, [[flow.apps]], vars and options -->

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

`[[flow.apps.devices]]` says which devices the flow runs on — see [Devices](#devices).

The `name` you give an app here is how any action with an `app` field (`launch`, `stop`, `clear_data`, `push_notification`, …) refers to it: `app = "app"`. Bundle ids also work. See [Multi-App Flows](#multi-app-flows).

### Launch-time permissions
<!-- summary: Grant app permissions before the app launches -->

An app entry may declare `permissions`. golem grants them **before the app is launched**, so they are in place from process start. A grant that fails is a warning, not a failure: the app then shows its permission prompt when it needs the permission.

```toml
[[flow.apps]]
name = "app"
permissions = { photos = "allow", camera = "deny" }
```

Keys use the cross-platform permission names (`camera`, `microphone`, `location`, `photos`, …). The value is a **mode**: `"allow"` or `"deny"` for any permission, plus `location = "always"` (background; `allow` is foreground) and `photos = "limited"`. To change a permission later in a flow, relaunch the app with the `launch` action's own `permissions =` map. [App permissions](actions-reference.md#app-permissions) owns the full permission and mode reference.

### Flow Options
<!-- summary: [flow.options]: timeouts, limits, recording, coverage, perf -->

```toml
[flow.options]
step_timeout = 5000                 # Base timeout (ms), default: 5000. See Timeout Multipliers.
max_steps = 10000                   # Fail the flow after this many steps (default: 10000)
max_runtime = "30m"                 # Fail the flow after this long (default: "1h"). "5m", "2h", "500ms"
max_device_wait = "30m"             # Fail if no device frees up within this time (default: wait forever). --max-device-wait overrides.
app_lifecycle = "reset"             # "reset" (default), "launch", "manual" — see Lifecycle
screenshot_on_failure = true        # Auto-capture screenshot on step failure (default: true)
record = true                       # Default every block to record (block can opt out with `record = false`)
coverage = "smart"                  # "smart" (default), "min", "full", "one" — see Coverage Strategies
create_if_missing = false           # Create a simulator/emulator when none matches (default: false) — see Hardware Axis
perf = true                         # Performance monitoring (default: true)
a11y = "relaxed"                    # Accessibility audit: "off", "critical", "relaxed" (default), "strict". --a11y overrides.
```

Thresholds are unset by default; the values below are examples. A `*_warn*` threshold adds a warning; a `*_error*` threshold fails the flow.

```toml
[flow.options]
perf_memory_warn_mb = 200.0         # App memory, MB
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0        # App CPU, %
perf_cpu_error_percent = 95.0
perf_threads_warn = 100             # Thread count
perf_threads_error = 200
perf_fd_warn = 200                  # Open file descriptors
perf_fd_error = 500
a11y_max_errors = 0                 # Fail the flow if cumulative a11y errors exceed this
a11y_max_warnings = 20              # Fail the flow if cumulative a11y warnings exceed this
a11y_min_confidence = 0.8           # Drop a11y findings below this confidence (0–1). Deterministic checks are 1.0.
```

### Accessibility Audit
<!-- summary: Per-block audit of the visible tree; levels and thresholds -->

After each block, Golem audits the **visible** UI tree for accessibility issues
(zero config, on by default at `relaxed`). Findings appear inline in the live run
and in every report format. They are warnings unless a flow sets the `a11y_*`
thresholds in [Flow Options](#flow-options). Levels: `off`, `critical` (tree
checks only), `relaxed` (default), `strict` (adds the screenshot contrast check +
an annotated screenshot).

Full guide — the checks, per-level thresholds, the confidence model, and how to
read the annotated screenshot — in **[accessibility.md](accessibility.md)**.

### Performance Monitoring
<!-- summary: Per-block memory, CPU, threads and fds; warn and error limits -->

Golem captures app performance metrics after each block (unless `--no-perf` or `perf = false`).

| Metric | Unit |
|--------|------|
| Memory | MB |
| CPU | % |
| Threads | count |
| File descriptors | count |
| Disk | MB |
| Network RX/TX | KB |

The `perf_*` thresholds in [Flow Options](#flow-options) act on memory, CPU, threads and file descriptors: crossing a warn threshold adds a warning, crossing an error threshold fails the flow.

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

`record = true` / `false` on a block overrides the flow's `record` default for that block (`--no-record` still wins).

`max_iterations = N` on a block fails the flow with `EF509` when the flow would run the block an (N+1)th time. It guards a block that loops through `next`, a branch `goto` or `[[data]]` rows: each row counts as one run. To leave a loop on purpose instead, branch on `_loop` (see [Branching](#branching)).

### Platform-Specific Blocks
<!-- summary: where: run a block only on devices that match -->

`where` runs a block only on devices that match it; on other devices the block is skipped:

```toml
[[block]]
name = "android_back"
where = { os = "android" }
steps = [
  { action = "press", button = "back" },
]
```

| Key | Matches |
|---|---|
| `os` | `"ios"` / `"android"` — the platform; any other value is an OS-version prefix (`"17"` matches 17.x) |
| `type` | `"phone"` / `"tablet"` |
| `physical` | `true` on a physical device, `false` on a simulator/emulator |

A device must match every key given. A skipped block's `next` and `[[block.branch]]` are not evaluated: the flow continues with the next block in document order.

### Branching
<!-- summary: Arms race on the screen; the first that matches goes to its block, else the else or fail -->

A block's `branch` says where the flow goes after the block's steps. Each arm has a condition and a `goto`:

```toml
[[block]]
name = "start"
steps = [{ action = "launch", app = "app" }]
branch = [
  { if_visible = "Allow notifications", goto = "permissions" },
  { if_visible = "Home", goto = "home" },
]
```

The app opens on either screen. golem waits for the first one to show, and goes to its block as soon as it shows.

**The arms race.** golem checks the arms after the block's steps finish (for a `for_each` block, after the last row; for a `run_flow` block, after the child flow returns). It first waits for the screen to settle, then reads one tree on each poll and checks each arm on it, in order. The first arm that matches wins.

- `if_visible` and `if_not_visible` wait for their condition up to the timeout of `assert_visible`. Set `timeout` (in ms) on the arm to change that. `timeout = 0` checks once.
- `if_var` arms are checked once, at once. A `timeout` on one is an error.

| Condition | Matches when |
|---|---|
| `if_visible = "…"` | an element with this text (glob) is on screen |
| `if_not_visible = "…"` | no element with this text is on screen |
| `if_var = "x"`, `equals = "…"` | the variable equals the string exactly |
| `if_var = "x"`, `matches = "…"` | the variable matches the glob pattern |
| `if_var = "x"`, `gte = N` | the variable, read as an integer, is ≥ `N` (an integer). A non-integer value never matches. |
| (no condition) | always: this arm is the else |

`if_var` without `equals`, `matches` or `gte` never matches.

**When no arm matches,** the flow takes the else: the last arm, if it has no condition, or the block's `next`. Without an else, the flow fails with `EF406`. The failure names the block, each arm and why it did not match, and the texts on the screen. A block with a branch never goes on to the next block in the file.

```toml
[[block]]
name = "pay"
steps = [{ action = "tap", on_text = "Pay" }]
next = "receipt"                                      # the else
branch = [{ if_visible = "Error", timeout = 0, goto = "retry" }]
```

golem checks the arms when it reads the flow:

- An arm with no condition that is not the last arm is an error: the arms after it never run.
- An arm with no condition and `next` on the same block is an error: both are the else.
- Two arms with the same condition and timeout give a warning: the second never wins.

### Block `next`
<!-- summary: Go to a named block after this one; with a branch, the else -->

Jump to a named block after completion (instead of falling through). On a block with a `branch`, `next` is the else: the flow goes there when no arm matches.

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
<!-- summary: One action with selectors, timeout and error handling -->

A step is a single action with optional selectors, timeouts, and error handling.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "assert_visible", on_text = "1", on_below = "Counter", timeout = 5000 }
{ action = "type", on_text = "Email", input = "hello@example.com" }
{ action = "read", on_right_of = "Status:", save_to = "status_value" }
```

### Selectors
<!-- summary: Selector keys on a step; prefer on_text -->

Find the element a step acts on. Prefer `on_text`; use `on_accessibility_label` only to test the a11y label.

| Selector | Description |
|----------|-------------|
| `on_text` | Match by visible text (glob, case-insensitive). **Preferred.** |
| `on_accessibility_label` | Match the accessibility label or the identifier (glob). |
| `on_below` / `on_above` / `on_right_of` / `on_left_of` | Position relative to an anchor element |

```toml
{ action = "tap", on_text = "Submit", on_below = "Counter" }
```

**See [Selectors](selectors.md)** for every selector, state filter and trait, the grouped `on = { … }` form, containment, and how a match is resolved.

### Step Options
<!-- summary: timeout, auto_scroll, if_fail, retry, save_to and other step fields -->

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | per-action | Max wait in ms. Overrides computed default. |
| `auto_scroll` | `false` | Scroll page to find element |
| `scroll_timeout` | — | With `scroll` or `auto_scroll`: stop scrolling after this many ms. The step `timeout` still applies. |
| `within` | — | With `scroll` or `auto_scroll`: scroll only inside the element this selector matches — see [`within`](selectors.md#within-scoping-a-scroll) |
| `keep_keyboard` | `false` | Leave the soft keyboard up: golem does not dismiss it before a tap or when it hides the target |
| `if_fail` | `"error"` | `"error"` (fail flow), `"warn"` (log + continue), `"ignore"` (silent continue) |
| `retry` | `0` | Retry count on failure |
| `retry_delay` | `1000` | Delay between retries (ms) |
| `save_to` | — | Save result to a variable |
| `secret` | — | With `save_to`: `true` masks the whole saved value in all output, and a list of paths, such as `["token"]`, masks only those — see [Secrets](#secrets) |
| `app` | — | Target a specific app (for multi-app flows) |

### Timeout Multipliers
<!-- summary: Each action's default timeout, as a multiple of step_timeout -->

Each action has a built-in multiplier applied to the base timeout (`step_timeout`, default 5000ms). Per-step `timeout` always overrides.

| Multiplier | Timeout (at 5s base) | Actions |
|------------|---------------------|---------|
| 1x | 5s | `screenshot`, `add_media`, `fail`, `load_fixture`, `push_notification`, `clear_data`, `press`, `set_dark_mode`, `set_location`, `hide_keyboard` |
| 2x | 10s | `tap`, `double_tap`, `backspace`, `clear_text`, `long_press`, `swipe`, `pinch`, `gesture`, `rotate`, `type`, `assert_visible`, `assert_not_visible`, `read`, `assert_alert`, `accept_alert`, `dismiss_alert`, and any action not listed here |
| 4x | 20s | `bash`, `run` |
| 5x | 25s | `launch`, `stop` |
| 6x | 30s | `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http`, `open_link`, `create_inbox` |
| 8x | 40s | `scroll` (12x with `within`) |
| 48x | 240s | `await_email` |

The `auto_scroll = true` option sets 8x on any action (12x with `within`).

Actions that take time by themselves (`long_press`, `swipe` through 3+ points, `gesture`, `rotate`, `type`, `backspace`, `clear_text`) get at least that time plus 2s: `max(multiplied, duration + 2s)`. For `type` and `backspace` the time is 500ms per character.

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
explicit_only = true        # Skip in the bulk sweep (see below)

[flow.options]
app_lifecycle = "manual"    # Don't restart the app

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "read", on_below = "Counter", on_index = 0, save_to = "counter_value" },
]
```

Variables listed in `[block.save_to]` propagate back to the parent. Override child variables with `[block.vars]`.

A subflow is a normal flow, so `golem run` (no path) would otherwise discover and run it standalone — redundant with the flows that embed it. Mark it `explicit_only = true` in `[flow]` to keep it out of the **bulk sweep** while still running it when you target it:

| Invocation | `explicit_only` flow |
|---|---|
| `golem run` (no path) | **skipped** — the bulk sweep |
| `golem run <dir>` (no `--tag`) | **skipped** |
| `golem run --tag login` (tag matches) | **runs** — a matching tag opts it in |
| `golem run --tag other` (no matching tag) | skipped |
| `golem run path/to/sub.test.toml` | **runs** — path given directly |
| `golem run 'e2e/**/*.test.toml'` (shell glob) | **runs** — the shell expands the glob to file paths before golem sees it, so a globbed path is indistinguishable from a typed one |

In short: `explicit_only` suppresses only the tag-less discovery sweep. Tag it to include it in specific `--tag` runs; name its path to run it directly. Set `app_lifecycle = "manual"` so the child inherits the parent's already-launched app (see [Lifecycle](#lifecycle-setup--teardown)).

## Reuse: Subflow vs Mixin vs Fixture
<!-- summary: When to use a subflow, a mixin or a fixture -->

Three ways to share pieces across flows, by what they contain:

| Concept | File / location | Contains | Reused via | Use when |
|---|---|---|---|---|
| **flow** | `x.test.toml` | `[flow]` + `[[block]]` | — (top-level unit) | a complete scenario |
| **subflow** | `x.test.toml`, usually `explicit_only = true` | a full `[flow]` | `run_flow` on a `[[block]]`; `[block.save_to]` propagates results back | reusing a whole scenario as a child (e.g. `login`) |
| **mixin** | `__mixins__/x.toml` | `[[step]]` only (no flow/block/vars) | [`load_mixin`](actions-reference.md#load_mixin--inline-a-reusable-step-sequence) action; steps inline into the block, per-call `vars` | reusing a step fragment that runs inside the caller's block state |
| **fixture** | `__fixtures__/x.toml` | `[vars]` only | [`load_fixture`](actions-reference.md#load_fixture--load-fixture-data) action; access as `${alias.key}` | reusing test **data** |

`__mixins__/` and `__fixtures__/` are excluded from flow discovery, so their files never run as tests on their own.

## Lifecycle: Setup & Teardown
<!-- summary: Setup is implicit: install, then app_lifecycle reset or launch -->

**There is no `[[setup]]` block.** A flow's setup is implicit and happens automatically before the first block:

1. **build and install** — once per app and device across the suite, cached (see [App Install](app-install.md)).
2. **app_lifecycle** — per flow, at flow start:
   - `reset` (default) — stop every app in `[[flow.apps]]`, then launch the first. Guarantees fresh state.
   - `launch` — launch the first app only if not already running. Preserves state.
   - `manual` — do nothing; the flow (or its parent) owns the app. `--start <block>` forces this.

Any additional setup you need (e.g. creating a user) is just normal steps, or a [mixin](#reuse-subflow-vs-mixin-vs-fixture) if shared.

**Subflows** never re-build or re-install (that layer isn't re-entered for a `run_flow` child), but the child **does** re-run `app_lifecycle` with *its own* setting — which is why reusable subflows set `app_lifecycle = "manual"` to inherit the parent's running app.

Cleanup after the flow belongs in [Teardown](#teardown).

## Teardown
<!-- summary: [[teardown]] steps run after the flow, even when it fails -->

Teardown blocks run after the flow completes, regardless of pass/fail — running **even when the flow fails** is the point: it cleans up external state (test data, created users) that a failed run would otherwise leak. Failures in teardown don't affect the test result (they surface as `Teardown:` warnings on the report). Teardown runs before the automatic device-state reset (dark mode, mocked location, recording), so it still sees the app as the flow left it.

```toml
[[teardown]]
steps = [
  { action = "screenshot", path = "/tmp/final.png" },
  { action = "stop", app = "app" },
]
```

A teardown step can be a [`load_mixin`](actions-reference.md#load_mixin--inline-a-reusable-step-sequence), for example to share a logout sequence between flows.

Skip teardown with `--no-teardown`.

## Data-Driven Tests
<!-- summary: [[data]] rows; for_each = "data" runs a block per row -->

A `[[data]]` table holds the rows, and a block iterates them with
`for_each = "data"`. The block runs once per row, and each row's fields are
read under the `${_each.<field>}` prefix:

```toml
[[data]]
user = "alice"

[[data]]
user = "bob"

[[block]]
for_each = "data"
steps = [
  { action = "type", on_text = "Search", input = "${_each.user}" },
  { action = "assert_visible", on_text = "${_each.user}" },
]
```

Only the `for_each` block repeats — surrounding blocks run once, and the
repeating block re-enters per row. An empty `[[data]]` table runs the block zero times.

Iteration is **block-level only**: rows parameterise steps inside a flow, not
whole flows. The block re-enters without relaunching the app, so a row that
leaves the app somewhere new is the next row's starting state — put anything
that must be reset into the block's own steps. To run a whole scenario per
case — each with a fresh app launch and its own pass/fail line — write it as
its own flow, or as a subflow invoked with different `[block.vars]`.

## Variables
<!-- summary: Set by --var, [flow.vars], rows, save_to or fixtures; read as ${name}; built-ins such as _device, _os -->

Set variables with `--var NAME=value` on the CLI, in `[flow.vars]`, from data rows, with a step's `save_to`, or from fixtures. Reference them as `${name}`:

```toml
[flow.vars]
base_url = "https://staging.example.com"

[[block]]
steps = [
  { action = "read", on_right_of = "Status:", save_to = "current_status" },
  { action = "bash", run = "echo ${current_status}", save_to = "result" },
]
```

When the CLI, `golem.toml` and the flow set the same name, see [Project config](#project-config-golemtoml) for which one wins.

### Built-in variables

golem reserves the `_` prefix and fills these in per step:

| Variable | Value |
|---|---|
| `_device` | Device name (`Pixel 9`) |
| `_os` | OS major version (`34`) |
| `_platform` | `android` / `ios` |
| `_type` | Device type (`phone`, `tablet`) |
| `_udid` | Device UDID |
| `_app` | App registry name of the step's app |
| `_hardware` | `virtual` on a sim/emulator, `real` on a physical device — same vocabulary as the `hardware` device constraint |
| `_loop` | 0-based count of times the current block has been entered |
| `_perf` | Last perf snapshot (object — see [Performance Monitoring](#performance-monitoring)) |

`_hardware`, `_loop` and `_perf` are also readable by a branch condition
(`[[block.branch]] if_var = "_hardware", equals = "real"`); the device and app
builtins resolve in `${…}` interpolation only.

## Secrets
<!-- summary: secret in a fixture or on a save_to step: golem masks the value in all output -->

golem shows variable values in its output: the step lines, the reports, the logs and the MCP results. Tag a value as secret where it enters the run, and golem replaces the value with its reference everywhere it writes. For example, `t:"staging-password"` shows as `t:"${qa.password}"`.

Tag a value in one of these places:

- **A fixture.** A root `secret` list names the keys of `[vars]` that are secret. See [`load_fixture`](actions-reference.md#load_fixture--load-fixture-data).
- **A step with `save_to`.** `secret = true` tags the whole saved value. `secret = ["token", "user.refresh_token"]` tags only those paths, for example the token of a test user that an HTTP step creates. If a path is not in the saved value, the step passes with the warning WP463, because the value under the real path is not masked.
- **`create_inbox`.** The inbox password (`pass`) is always secret.

To keep a secret out of every file, get it from the environment or a secret manager with a step:

```toml
{ action = "bash", run = "op read op://qa/password", save_to = "pw", secret = true }
```

golem masks the value, not the variable. A string made from a secret, such as `"Bearer ${user.token}"`, shows as `Bearer ${user.token}`, and a response that sends the token back is masked too. golem masks a secret of any length. A short secret, such as a PIN, also masks the same text elsewhere in the output: the output is harder to read, but the secret does not show. For an object such as the result of `run`, name the paths: `secret = true` also masks fields such as `exit_code`.

The limits:

- golem cannot mask pixels. A screenshot or a screen recording shows a secret that the app displays. Password fields usually show dots.
- golem cannot mask a changed form of a value: base64, URL encoding, a hash, or a value split across lines.
- Masking hides a value. It does not stop a step from using the value: a step can still send it with `bash` or `post_http`.
- Use test accounts, not real users.

## Fake Data Generators
<!-- summary: ${fake:…} in short; help("fake") has the rest -->

`${fake:…}` generates random but valid test data: `email`, `password`, `uuid`, `number`, `one_of`, `sentence`, `timestamp`, `phone`, and the structured `person`, `address` and `credit_card`. `--seed <N>` replays the same values.

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
```

**See [Fake Data Generators](fake-data.md)** for every generator, its parameters and fields.

## Multi-App Flows
<!-- summary: Several apps in one flow; switch with launch app = "…" -->

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

## Devices

Each `[[flow.apps]]` entry lists device constraints in `[[flow.apps.devices]]`: OS and version (`os`), shape (`type`), virtual or physical (`hardware`), or an exact device (`name`). golem turns the constraints into device slots, and the `coverage` strategy decides how many devices run the flow. For each slot it picks a free booted device that matches, boots a shut-down one if no match is booted, and creates one only when nothing matches and `create_if_missing` is set. With no `[[flow.apps.devices]]` at all, the flow runs on whatever is already booted.

### Coverage Strategies
<!-- summary: How multi-valued device axes expand into runs: smart, min and more -->

`coverage` in [Flow Options](#flow-options) controls how multi-valued `[[flow.apps.devices]]` axes expand into FlowRuns.

| Strategy | Behaviour |
|---|---|
| `smart` (default) | Ticks every axis value, and uses more devices when they are free. |
| `min` | Fewest devices that tick every axis value. Every planned run runs. |
| `full` | Cartesian product — one FlowRun per (os × type × …) combination. Use when every combo needs independent validation. |
| `one` | The first successful run ends the group. Local smoke testing. Tolerates underspec (`ios:latest:2` with only one version available). |

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

### Hardware Axis (virtual / real)
<!-- summary: hardware = "virtual" or "real", or either -->

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

### Pinning a Specific Device by Name
<!-- summary: name = "…": pin one device by its display name -->

```toml
[[flow.apps.devices]]
name = "iPhone 15"
```

`name` pins an exact device display name (as shown by `golem devices` / `xcrun simctl list` / `adb devices -l`). Use this when you have a customised simulator or a specific physical device the flow must target.

With `create_if_missing`, an unknown name errors instead of creating a simulator.

### Auto-Boot Behaviour
<!-- summary: golem boots a matching shut-down device; headless on Android -->

When no booted device matches a slot but a matching simulator/emulator is shut down, golem boots it and waits until it is ready.

Emulators that golem boots on Android run headless — no window. To watch a device, boot it yourself before the run (golem reuses a booted device instead of starting another), or on iOS open `Simulator.app`, which shows the simulators golem boots.

## Project config (`golem.toml`)
<!-- summary: golem.toml: project defaults that every flow inherits -->

`golem.toml` sits at the project root (golem walks up from the working
directory to find it) and holds the defaults every flow inherits. A flow
always wins over the project for anything it states itself.

```toml
[vars]                      # referenced in any flow as ${base_url}
base_url = "https://staging.example.com"

[options]                   # defaults for every flow's [flow.options]
step_timeout = 8000
record = true
max_device_wait = "30m"     # queue-wait cap; --max-device-wait beats it

[[apps]]                    # app registry — flows reference by name
name = "app"
bundle = "com.example.myapp"
install_script = "scripts/install.sh"

[[teardown]]                # appended to every flow's own teardown
steps = [
  { action = "bash", run = "scripts/cleanup.sh" },
]

[device_settings]           # OS-level tweaks applied once per device session
android = { "secure.long_press_timeout" = "400" }
```

| Section | Merge rule |
|---|---|
| `[vars]` | Flow `[flow.vars]` of the same name wins |
| `[options]` | Per field: a flow's `[flow.options]` value wins, others fall through |
| `[[apps]]` | Flows inherit `bundle` / `install_script` / `install_timeout_ms` / `devices` by app name |
| `[[teardown]]` | Runs after the flow's own teardown, and runs even when the flow failed |
| `[device_settings]` | Applied to the device before any flow runs — not a flow-level concept |

CLI flags beat both: `--var` overrides a project or flow var, and the
recording flags override `record` at every level.
