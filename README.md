# Golem

Cross-platform mobile UI testing framework. Write tests once in TOML, run on iOS and Android simultaneously.

## Quick Start

```bash
# Initialize a project
golem init

# Create a test flow
golem create login

# Run tests
golem run flows/login.test.toml

# Run all flows in a directory
golem run flows/

# List connected devices
golem devices

# Inspect the live UI tree
golem tree
```

## CLI Reference

### `golem run`

Run one or more test flows.

```bash
golem run [FILES...] [OPTIONS]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `FILES...` | Flow files or directories. If empty, auto-discovers from current directory. |

**Options:**

| Flag | Description |
|------|-------------|
| `--platform <ios\|android>` | Force a single platform (overrides flow device config) |
| `--tag <TAG>` | Filter flows by tag. Repeatable. Use `\|` within a value for OR. |
| `--var <KEY=VALUE>` | Set a variable. Repeatable. (not yet wired) |
| `--output <FORMAT[:FILE]>` | Output format. Repeatable. Default: `human`. See [Output Formats](#output-formats). |
| `--seed <N>` | Deterministic seed for fake data generation (not yet wired) |
| `--start <BLOCK>` | Start execution at a named block (skips app lifecycle, assumes app in correct state) |
| `--max-concurrency <N>` | Max parallel devices (not yet implemented) |
| `--record` | Enable auto screen recording (not yet implemented) |
| `--no-clean` | Skip app data clear between flows (not yet implemented) |
| `--no-teardown` | Skip teardown blocks (not yet wired) |
| `--keep-devices` | Keep devices running after completion (not yet wired) |
| `--no-perf` | Disable performance capture |
| `--verbose` | Show substeps: scroll coordinates, strategies, tree stats |
| `--debug` | Show driver diagnostics: WebKit/CDP connection details |

**Examples:**

```bash
# Run on Android only
golem run flows/ --platform android

# Run with variables
golem run flows/login.test.toml --var EMAIL=test@example.com --var PASSWORD=secret

# Multiple output targets
golem run flows/ --output human --output json:report.json --output junit:results.xml

# Filter by tag
golem run flows/ --tag smoke
golem run flows/ --tag "auth|login"

# Verbose mode for debugging scroll behavior
golem run flows/scroll.test.toml --verbose
```

### `golem tree`

Inspect the live UI element hierarchy from a running device.

```bash
golem tree [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--platform <ios\|android>` | Filter by platform |
| `--device <NAME>` | Filter by device name or UDID (substring match) |
| `--bundle <ID>` | App bundle ID (default: `fail.golem.test`) |
| `--full` | Show full tree without viewport filtering |
| `--json` | Output as JSON |
| `--verbose` | Show metadata: CDP status, enrichment, keyboard, safe area |

### `golem devices`

List all connected simulators, emulators, and physical devices.

### `golem init`

Scaffold a new project: creates `golem.toml`, `flows/`, `__fixtures__/`, `__mixins__/`, and `.golem/`.

### `golem create <name>`

Create a new flow template at `flows/<name>.test.toml`.

---

## Output Formats

Specify with `--output FORMAT[:FILE]`. Multiple formats can run simultaneously.

### `human` (default)

Real-time colored output streamed to stderr. Shows step-by-step progress with timing, pass/fail symbols, and a suite summary.

```
▶ tap.test
  ── tap_interactions ──
  [1][tap_interactions][0] tap on_text="+"
      ✓  [1200ms]
  [2][tap_interactions][1] assert_visible on_text="1" on_below="Counter"
      ✓  [320ms]

  ✓ PASSED  tap.test  [2.1s]
```

Step labels read as `[global_step][block_name][step_within_block]`. With data-driven tests or `for_each` iterations, the block name includes the iteration: `[3][login:0][1]`, `[6][login:1][1]`.

With `--verbose`, shows substeps and tree stats. The `{3 trees, 186~190 nodes}` suffix shows how many UI hierarchy fetches the step needed and the node count range across those fetches. Higher tree counts indicate retries or scroll iterations; changing node counts suggest the UI was updating.

Scroll substeps show the strategy number (1-5 per direction), swipe coordinates, and outcome. Strategies vary the swipe distance and position to handle different scroll contexts — strategy 1 is a full-page swipe, higher numbers try shorter or offset swipes to handle inner scrollable containers.
```
  [3][tap_interactions][2] tap on_text="+"
      ∙ element_resolved "+" bounds=(48,161,43,36) tap=(69,179)
      ∙ tap (69,179)
      ✓  [2126ms] {3 trees, 186~190 nodes}
  [5][scroll_test][1] read on_right_of="Orientation:" auto_scroll
      ∙ [scroll] ↓ strategy 1 (540,1560)→(540,256) → page scrolled
      ∙ [scroll] ↓ strategy 2 (540,2160)→(540,840) → found at (550,459)
      ∙ element_resolved "Portrait" bounds=(200,459,80,18) tap=(240,468)
      ✓  [8234ms] {3 trees, 187~188 nodes}
```

### `json` or `json:<file>`

Structured JSON with suite summary, per-flow results, step details, substeps, and performance snapshots. Without a file path, outputs to stdout.

### `junit` or `junit:<file>`

JUnit XML for CI systems (Jenkins, GitHub Actions, GitLab CI). Each flow maps to a `<testsuite>`, each step to a `<testcase>`. Without a file path, outputs to stdout.

### `toon`

Token-optimized format for LLM analysis. ~40-60% smaller than human format.

```
S:tap_test d:450 seed:847291036
 +tap:+ 45 t:3/142
 +assert_visible:1 120
R:PASS 2/0/0
```

---

## Test Structure

Tests are written in TOML. A `.test.toml` file defines a **flow** — the top-level unit of execution.

### Flow

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

#### Flow Options

```toml
[flow.options]
step_timeout = 30000                # Default timeout per step (ms) — not yet wired
max_steps = 10000                   # Safety limit
max_runtime = "30m"                 # "5m", "2h", "500ms"
app_lifecycle = "reset"             # "reset" (default), "launch", "manual"
screenshot_on_failure = true        # Not yet wired (hardcoded true)
screenshot_dir = "/tmp/shots"       # Not yet wired (hardcoded .golem/screenshots)
record = true                       # Not yet wired
recording_dir = "/tmp/videos"       # Not yet wired (hardcoded .golem/recordings)
perf = true                         # Performance monitoring (default: true)
perf_memory_warn_mb = 200.0
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0
perf_cpu_error_percent = 95.0
```

#### Performance Monitoring

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

### Block

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

#### Platform-Specific Blocks

Skip blocks that don't apply to the current device:

```toml
[[block]]
name = "android_back"
where = { os = "android" }
steps = [
  { action = "press", button = "back" },
]
```

#### Branching

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

#### Block `next`

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

### Step

A step is a single action with optional selectors, timeouts, and error handling.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "assert_visible", on_text = "1", on_below = "Counter", timeout = 5000 }
{ action = "type", on_text = "Email", input = "hello@example.com" }
{ action = "read", on_right_of = "Status:", save_to = "status_value" }
```

#### Selectors

Find elements by visible text, accessibility labels, position, or state:

| Selector | Description |
|----------|-------------|
| `on_text` | Match by visible text (glob pattern, case-insensitive) |
| `on_accessibility_label` | Match by accessibility identifier |
| `on_index` | Match the Nth element (0-based) |
| `on_enabled` | Filter by enabled state (`true`/`false`) |
| `on_checked` | Filter by checked state (`true`/`false`) |
| `on_clickable` | Filter by clickability |
| `on_below` | Element must be below this anchor text |
| `on_above` | Element must be above this anchor text |
| `on_right_of` | Element must be right of this anchor text |
| `on_left_of` | Element must be left of this anchor text |

#### Grouped Selector Syntax

For complex queries, use `on = {}` instead of flat `on_*` fields:

```toml
# Flat (simple cases)
{ action = "tap", on_text = "Submit", on_below = "Counter" }

# Grouped (complex selectors, nested anchors)
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }

# Nested anchor with its own selectors
{ action = "assert_visible", on = { text = "Portrait", right_of = { text = "Orientation:" } } }

# Traits filtering
{ action = "assert_visible", on = { text = "Submit", traits = ["button", "has_text"] } }
```

#### Step Options

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | `step_timeout` | Max wait in ms |
| `auto_scroll` | `false` | Scroll page to find element |
| `max_scrolls` | — | Limit scroll attempts |
| `if_fail` | `"error"` | `"error"` (fail flow), `"warn"` (log + continue), `"ignore"` (silent continue) |
| `retry` | `0` | Retry count on failure |
| `retry_delay` | `1000` | Delay between retries (ms) |
| `save_to` | — | Save result to a variable |
| `app` | — | Target a specific app (for multi-app flows) |

### Subflow

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

### Teardown

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

### Data-Driven Tests

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

### Variables

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

### Fake Data Generators

Generate realistic test data with the `fake:` prefix in variable declarations. Generators produce random but valid values. Deterministic replay via `--seed` is not yet wired.

```toml
[flow.vars]
email = "fake:email"
user = "fake:person(country=JP)"
addr = "fake:address(country=GB)"
card = "fake:credit_card(brand=visa)"
```

Access structured fields with dot notation: `${user.name}`, `${addr.city}`, `${card.number}`.

#### Simple generators

| Generator | Output | Parameters |
|-----------|--------|------------|
| `fake:email` | `abc123@example.com` | `prefix`, `domain` |
| `fake:first_name` | Random first name | — |
| `fake:last_name` | Random last name | — |
| `fake:password` | Random password | `length` (default 12), `symbols` (default true) |
| `fake:uuid` | UUID v4 | — |
| `fake:number` | Random integer string | `min` (default 0), `max` (default 100) |
| `fake:sentence` | Simple English sentence | — |
| `fake:timestamp` | ISO 8601 within last year | — |
| `fake:phone` | Country-formatted phone | `country` (ISO code), `format` (`#` = digit) |
| `fake:city` | City name | `country`, `region` |
| `fake:postcode` | Postal code | `country` |
| `fake:street` | Street address | `country` |

#### Structured generators

These return objects with multiple fields.

**`fake:person`** — `country` parameter affects name ordering and phone format.

| Field | Example |
|-------|---------|
| `first` | Yuki |
| `last` | Tanaka |
| `name` | Tanaka Yuki (family-first for JP/CN/KR) |
| `email` | yuki.tanaka@example.com |
| `phone` | +81-90-1234-5678 |

**`fake:address`** — Parameters: `country`, `state`, `region`.

| Field | Example |
|-------|---------|
| `street` | 42 Baker Street |
| `city` | London |
| `state` | England |
| `postcode` | SW1A 1AA |
| `country` | United Kingdom |
| `country_code` | GB |

**`fake:credit_card`** — Generates Luhn-valid card numbers. Parameters: `brand` (visa/mastercard/amex/discover), `provider` (stripe/adyen/square/etc.), `status`.

| Field | Example |
|-------|---------|
| `number` | 4532015112830366 |
| `expiry` | 03/28 |
| `cvv` | 123 |
| `brand` | visa |
| `status` | (empty if approved) |

Status options without provider: `approved`, `declined:invalid_number`, `declined:expired`, `declined:invalid_cvv`, `threeds`. Provider-specific statuses vary.

#### Cross-references

Later generators can reference earlier variables:

```toml
[flow.vars]
addr = "fake:address(country=JP)"
phone = "fake:phone(country=${addr.country_code})"
person = "fake:person(country=${addr.country_code})"
```

### Multi-App Flows

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

---

## Actions Reference

### Interaction

#### `tap` — Tap an element

Find an element matching the selectors and tap its center.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "tap", on_text = "+", timeout = 5000 }
{ action = "tap", on = { text = "OK", below = "Confirm?" } }
{ action = "tap", on_accessibility_label = "Increment" }
```

Supports all selectors, `auto_scroll`, `timeout`, `if_fail`, `retry`.

#### `doubleTap` — Double-tap an element

Two rapid taps (40ms apart) at the element center.

```toml
{ action = "doubleTap", on_text = "Zoom" }
```

Same selectors and options as `tap`.

#### `type` — Type text into an element

Taps the element to focus it, then types the `input` string.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |

#### `backspace` — Delete characters

Taps the element to focus, then sends backspace key presses.

```toml
{ action = "backspace", on_text = "Email", count = 5 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `count` | `1` | Number of backspace presses |

#### `long_press` — Long press an element

Press and hold at the element center.

```toml
{ action = "long_press", on_text = "Item", duration = 2000 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `duration` | `1000` | Hold duration in ms |

#### `swipe` — Swipe gesture

Direction-based swipe or path-based with start/end points.

```toml
# Direction-based
{ action = "swipe", direction = "down" }
{ action = "swipe", direction = "left" }

# Path-based with selectors
{ action = "swipe", start = { text = "Slider" }, end = { text = "Max" }, duration = 500 }
```

| Field | Description |
|-------|-------------|
| `direction` | `"up"`, `"down"`, `"left"`, `"right"` |
| `start` | Start position (SelectorGroup) |
| `end` | End position (SelectorGroup) |
| `points` | Array of intermediate points for complex paths |
| `duration` | Gesture duration in ms |

#### `scroll` — Scroll until element found

Scrolls the page (or a container) until the target element is visible.

```toml
# Scroll page to find element
{ action = "scroll", to = { text = "Item 25" }, timeout = 60000 }

# Scroll within a specific container
{ action = "scroll", to = { text = "Item 45" }, within = { below = "Scroll List" }, timeout = 60000 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `direction` | `"down"` | Scroll direction |
| `within` | — | Constrain scrolling to an element's bounds |
| `max_scrolls` | — | Limit iterations |
| `timeout` | — | Overall scroll timeout |

#### `pinch` — Pinch zoom gesture

Two-finger pinch centered on an element or coordinates.

```toml
{ action = "pinch", scale = 2.0, duration = 500 }     # Zoom in
{ action = "pinch", scale = 0.5, duration = 500 }     # Zoom out
```

| Field | Default | Description |
|-------|---------|-------------|
| `scale` | — | `>1.0` = zoom in, `<1.0` = zoom out |
| `velocity` | `5.0` | Scale factor per second |

#### `gesture` — Multi-touch gesture

Arbitrary multi-finger gesture with explicit paths.

```toml
[[block.steps]]
action = "gesture"
duration = 300

[[block.steps.fingers]]
points = [
  { x = 200, y = 400 },
  { x = 200, y = 200 },
]

[[block.steps.fingers]]
points = [
  { x = 200, y = 200 },
  { x = 200, y = 400 },
]
```

| Field | Default | Description |
|-------|---------|-------------|
| `fingers` | — | Array of finger paths, each with `points` |
| `duration` | `300` | Duration per finger (ms) |

#### `hide_keyboard` — Dismiss keyboard

Dismiss the on-screen keyboard. No-op if no keyboard is visible.

```toml
{ action = "hide_keyboard" }
```

### Assertions

#### `assert_visible` — Assert element exists

Verify an element matching the selectors is on screen. Aliases: `assert_text`, `assert_enabled`, `assert_checked`.

```toml
{ action = "assert_visible", on_text = "Welcome" }
{ action = "assert_visible", on_text = "1", on_below = "Counter" }
{ action = "assert_visible", on = { text = "Submit", traits = ["button"] } }

# With auto-scroll for off-screen elements
{ action = "assert_visible", on_text = "Item 0", auto_scroll = true, timeout = 60000 }

# Check enabled state
{ action = "assert_visible", on_text = "Submit", on_enabled = true }

# Check checked state
{ action = "assert_checked", on_accessibility_label = "agree-checkbox", on_checked = true }
```

#### `assert_not_visible` — Assert element absent

Verify no element matches the selectors.

```toml
{ action = "assert_not_visible", on_text = "Error" }
{ action = "assert_not_visible", on_text = "Loading", timeout = 10000 }
```

#### `assert_alert` — Assert alert is displayed

Verify an alert/dialog is showing. Optionally match alert text with a glob pattern.

```toml
{ action = "assert_alert" }
{ action = "assert_alert", on_text = "Are you sure*" }
```

### Wait

#### `wait` — Wait for element to appear

Poll until an element matching selectors becomes visible.

```toml
{ action = "wait", on_text = "Ready", timeout = 15000 }
```

#### `wait_not` — Wait for element to disappear

Poll until no element matches the selectors.

```toml
{ action = "wait_not", on_text = "Loading...", timeout = 10000 }
```

### Reading

#### `read` — Read element text

Find an element and capture its text into a variable.

```toml
{ action = "read", on_right_of = "Status:", save_to = "status" }
{ action = "read", on_below = "Counter", on_index = 0, save_to = "count" }
```

| Field | Description |
|-------|-------------|
| `save_to` | Variable name to store the text value |

### App Lifecycle

#### `launch` — Launch or foreground an app

Bring an app to the foreground. Does not restart if already running. Use `restart = true` for a cold start.

```toml
{ action = "launch", app = "app" }
{ action = "launch", app = "companion" }
{ action = "launch", app = "app", restart = true }   # Kill and relaunch
```

| Field | Default | Description |
|-------|---------|-------------|
| `app` | — | App name (as defined in `[[flow.apps]]`) |
| `restart` | `false` | Stop app first, then launch fresh |

#### `stop` — Terminate an app

```toml
{ action = "stop", app = "app" }
```

#### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

### Device Controls

#### `rotate` — Set device orientation

```toml
{ action = "rotate", orientation = "landscape" }
{ action = "rotate", orientation = "portrait" }
```

#### `dark_mode` — Toggle dark mode

```toml
{ action = "dark_mode", enabled = true }
{ action = "dark_mode", enabled = false }
```

#### `set_location` — Set GPS coordinates

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

#### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

#### `grant_permission`, `revoke_permission` — Manage app permissions

```toml
{ action = "grant_permission", app = "app", permission = "camera" }
{ action = "revoke_permission", app = "app", permission = "location" }
```

### Capture

#### `screenshot` — Take screenshot

```toml
{ action = "screenshot" }
{ action = "screenshot", path = "/tmp/dark-mode.png" }
```

#### `start_recording`, `stop_recording` — Screen recording

```toml
{ action = "start_recording", path = "login_flow" }
# ... test steps ...
{ action = "stop_recording", path = "/tmp/login.mp4" }
```

#### `add_media` — Push media to device

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

### Alerts

#### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes) on the current alert.

```toml
{ action = "accept_alert" }
```

#### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No) on the current alert.

```toml
{ action = "dismiss_alert" }
```

### External

#### `open_link` — Open URL or deep link

```toml
{ action = "open_link", url = "https://example.com" }
{ action = "open_link", url = "myapp://profile/123" }
```

#### `push_notification` — Send push notification

```toml
{ action = "push_notification", title = "New message", body = "Hello!" }
```

#### `bash` — Run shell command

Execute a command via `sh -c`. Fails if exit code is non-zero.

```toml
{ action = "bash", run = "curl -s https://api.example.com/reset" }
{ action = "bash", run = "echo $ENV_VAR", save_to = "result" }
```

#### `run` — Run project script

Execute a script relative to the project root or flow directory. Rejects path traversal (`..`).

```toml
{ action = "run", script = "/scripts/seed_db.sh" }
{ action = "run", script = "/scripts/setup.sh", args = ["staging", "verbose"], save_to = "output" }
```

Leading `/` = relative to project root. No leading `/` = relative to flow file directory.

#### `await_email` — Poll IMAP inbox

Wait for an email matching filters, with optional regex extraction.

```toml
{ action = "await_email", inbox = "test_inbox", subject = "Verify*", timeout = 30000, save_to = "email" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `inbox` | — | Variable namespace with `imap_host`, `imap_port`, `user`, `pass` |
| `to` | — | Glob filter for recipient |
| `subject` | `"*"` | Subject glob pattern |
| `extract` | — | Table of field names to regex patterns |
| `timeout` | `30000` | Polling timeout (ms) |

#### `load_fixture` — Load fixture data

Load variables from a TOML file in `__fixtures__/`.

```toml
{ action = "load_fixture", fixture = "users", as = "test_user" }
# Access as ${test_user.email}, ${test_user.name}, etc.
```

#### `http_get`, `http_post`, `http_put`, `http_patch`, `http_delete` — HTTP requests

```toml
{ action = "http_get", url = "https://api.example.com/status", save_to = "response" }
{ action = "http_post", url = "https://api.example.com/reset", body = "{\"force\": true}" }
{ action = "http_get", url = "https://api.example.com/data", headers = { Authorization = "Bearer ${token}" } }
```

Fails on non-2xx status codes.

### Flow Control

#### `fail` — Fail the flow immediately

```toml
{ action = "fail", on_text = "Unexpected state reached" }
```

Useful in conditional branches to mark unreachable paths.
