# Actions Reference

*Every word the golem knows.*

← [Back to README](../README.md) · See [Test Structure](test-structure.md) for selectors, steps, and flow anatomy.

## Contents

- [Interaction](#interaction)
  - [tap](#tap--tap-an-element)
  - [double_tap](#double_tap--double-tap-an-element)
  - [type](#type--type-text-into-an-element)
  - [backspace](#backspace--delete-characters)
  - [long_press](#long_press--long-press-an-element)
  - [swipe](#swipe--swipe-gesture)
  - [scroll](#scroll--scroll-until-element-found)
  - [pinch](#pinch--pinch-zoom-gesture)
  - [gesture](#gesture--multi-touch-gesture)
  - [rotate](#rotate--rotate-gesture)
  - [hide_keyboard](#hide_keyboard--dismiss-keyboard)
- [Assertions](#assertions)
  - [assert_visible](#assert_visible--wait-for--assert-element-exists)
  - [assert_not_visible](#assert_not_visible--wait-for--assert-element-absent)
  - [assert_alert](#assert_alert--assert-alert-is-displayed)
- [Reading](#reading)
  - [read](#read--read-element-text)
- [App Lifecycle](#app-lifecycle)
  - [launch](#launch--launch-or-foreground-an-app)
  - [stop](#stop--terminate-an-app)
  - [clear_data](#clear_data--clear-app-data)
- [Device Controls](#device-controls)
  - [set_dark_mode](#set_dark_mode--set-dark-mode)
  - [set_location](#set_location--set-gps-coordinates)
  - [press](#press--press-hardware-button)
  - [grant_permission / revoke_permission](#grant_permission-revoke_permission--manage-app-permissions)
- [Capture](#capture)
  - [screenshot](#screenshot--take-screenshot)
  - [recording](#screen-recording--per-block-via-record--true)
  - [add_media](#add_media--push-media-to-device)
- [Alerts](#alerts)
  - [accept_alert](#accept_alert--accept-dialog)
  - [dismiss_alert](#dismiss_alert--dismiss-dialog)
- [External](#external)
  - [open_link](#open_link--open-url-or-deep-link)
  - [push_notification](#push_notification--deliver-a-push-to-the-app-under-test)
  - [bash](#bash--run-shell-command)
  - [run](#run--run-project-script)
  - [create_inbox](#create_inbox--provision-a-disposable-email-inbox)
  - [await_email](#await_email--poll-imap-inbox)
  - [load_fixture](#load_fixture--load-fixture-data)
  - [*_http](#get_http-post_http-put_http-patch_http-delete_http--http-requests)
- [Flow Control](#flow-control)
  - [fail](#fail--fail-the-flow-immediately)

> The canonical list of action keywords is the dispatch match in [`golem-runner/src/actions.rs`](../golem-runner/src/actions.rs). If you add a handler there, document it here.

## Interaction

### `tap` — Tap an element

Find an element matching the selectors and tap its center.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "tap", on_text = "+", timeout = 5000 }
{ action = "tap", on = { text = "OK", below = "Confirm?" } }
{ action = "tap", on_accessibility_label = "Increment" }
```

Supports all selectors, `auto_scroll`, `timeout`, `if_fail`, `retry`.

### `double_tap` — Double-tap an element

Two rapid taps (40ms apart) at the element center.

```toml
{ action = "double_tap", on_text = "Zoom" }
```

Same selectors and options as `tap`.

### `type` — Type text into an element

Taps the element to focus it, then types the `input` string.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |

### `backspace` — Delete characters

Taps the element to focus, then sends backspace key presses.

```toml
{ action = "backspace", on_text = "Email", count = 5 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `count` | `1` | Number of backspace presses |

### `long_press` — Long press an element

Press and hold at the element center.

```toml
{ action = "long_press", on_text = "Item", duration = 2000 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `duration` | `1000` | Hold duration in ms |

### `swipe` — Swipe gesture

`swipe` is the **raw** gesture primitive — one direction-based swipe or a path-based gesture defined by `start` / `end` (and optional `points` for 3+ point paths). Use `scroll` instead when you want golem to *keep* swiping until an element appears.

```toml
# Direction-based — single swipe from a sensible default origin
{ action = "swipe", direction = "down" }
{ action = "swipe", direction = "left" }

# Path-based with selectors — start and end resolve to element centres
{ action = "swipe", start = { text = "Slider" }, end = { text = "Max" }, duration = 500 }

# Anchored to a container (no `within` for swipe — use `start` / `end`)
{ action = "swipe",
  start = { below = "Scroll List" },
  end   = { below = "Scroll List", y = "30%" } }
```

| Field | Description |
|-------|-------------|
| `direction` | `"up"`, `"down"`, `"left"`, `"right"` |
| `start` | Start position (SelectorGroup: text / accessibility_label / below / above + optional x / y offsets) |
| `end` | End position (SelectorGroup) |
| `points` | Array of intermediate points for complex paths |
| `duration` | Gesture duration in ms |

> **Note:** `within` is **not** consumed by `swipe` — only by `scroll` and by any step with `auto_scroll = true`. Use `start` / `end` to anchor a swipe inside a container. A `within` set on a swipe (or other unsupported action) emits a `[lint]` warning at plan time; a future `--validate` mode will reject it as an error.

### `scroll` — Scroll until element found

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

### `pinch` — Pinch zoom gesture

Two-finger pinch centered on an element or coordinates.

```toml
{ action = "pinch", scale = 2.0, duration = 500 }     # Zoom in
{ action = "pinch", scale = 0.5, duration = 500 }     # Zoom out
```

| Field | Default | Description |
|-------|---------|-------------|
| `scale` | — | `>1.0` = zoom in, `<1.0` = zoom out |
| `velocity` | `5.0` | Scale factor per second |

### `gesture` — Multi-touch gesture

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

### `rotate` — Rotate gesture

A two-finger **rotation gesture** centered on an element (or screen). `rotate` is a multi-touch gesture, **not** a device-orientation change — programmatic device orientation is [unsupported](unsupported.md).

Two fingers orbit a center point — resolved from an element selector, or from explicit `x` / `y` coordinates.

```toml
{ action = "rotate", on_text = "Map", rotation = 90.0 }    # rotate 90° clockwise
{ action = "rotate", on_text = "Map", rotation = -45.0 }   # 45° counter-clockwise
```

| Field | Default | Description |
|-------|---------|-------------|
| `rotation` | — (required) | Degrees to rotate. Positive = clockwise, negative = counter-clockwise. |
| `velocity` | `180.0` | Rotation speed in degrees per second |

### `hide_keyboard` — Dismiss keyboard

Dismiss the on-screen keyboard. No-op if no keyboard is visible.

```toml
{ action = "hide_keyboard" }
```

## Assertions

### `assert_visible` — Wait for / assert element exists

Poll the hierarchy until an element matching the selectors is on screen, or `timeout` elapses (default 10s). Use a short `timeout` for instantaneous checks, a long one for waits. The assertion is driven by the selectors — add `on_enabled` / `on_checked` to assert state, not just presence.

```toml
{ action = "assert_visible", on_text = "Welcome" }
{ action = "assert_visible", on_text = "1", on_below = "Counter" }
{ action = "assert_visible", on = { text = "Submit", traits = ["button"] } }

# With auto-scroll for off-screen elements
{ action = "assert_visible", on_text = "Item 0", auto_scroll = true, timeout = 60000 }

# Check enabled state
{ action = "assert_visible", on_text = "Submit", on_enabled = true }

# Check checked state
{ action = "assert_visible", on_accessibility_label = "agree-checkbox", on_checked = true }
```

### `assert_not_visible` — Wait for / assert element absent

Poll the hierarchy until no element matches the selectors, or `timeout` elapses (default 10s).

```toml
{ action = "assert_not_visible", on_text = "Error" }
{ action = "assert_not_visible", on_text = "Loading", timeout = 10000 }
```

### `assert_alert` — Assert alert is displayed

Verify an alert/dialog is showing. Optionally match alert text with a glob pattern.

```toml
{ action = "assert_alert" }
{ action = "assert_alert", on_text = "Are you sure*" }
```

## Reading

### `read` — Read element text

Find an element and capture its text into a variable.

```toml
{ action = "read", on_right_of = "Status:", save_to = "status" }
{ action = "read", on_below = "Counter", on_index = 0, save_to = "count" }
```

| Field | Description |
|-------|-------------|
| `save_to` | Variable name to store the text value |

## App Lifecycle

### `launch` — Launch or foreground an app

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

### `stop` — Terminate an app

```toml
{ action = "stop", app = "app" }
```

### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

## Device Controls

### `set_dark_mode` — Set dark mode

```toml
{ action = "set_dark_mode", enabled = true }
{ action = "set_dark_mode", enabled = false }
```

### `set_location` — Set GPS coordinates

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

### `grant_permission`, `revoke_permission` — Manage app permissions

Pre-grants (or revokes) a permission via the platform's privacy
database (`pm grant` on Android, `simctl privacy` on iOS sims). The
`app` field is required and must name an entry from `[[flow.apps]]`
or the project's `golem.toml` so the bundle id can be resolved.

```toml
{ action = "grant_permission", app = "app", permission = "camera" }
{ action = "revoke_permission", app = "app", permission = "location" }
```

**Cross-platform shorthands.** One vocabulary, mapped per platform:

| Shorthand        | Android (`pm grant`)                                      | iOS (`simctl privacy`) |
|------------------|-----------------------------------------------------------|------------------------|
| `camera`         | `CAMERA`                                                  | `camera`               |
| `microphone`     | `RECORD_AUDIO`                                            | `microphone`           |
| `location`       | `ACCESS_FINE_LOCATION` (foreground only)                  | `location`             |
| `location-always`| `ACCESS_FINE_LOCATION` + `ACCESS_BACKGROUND_LOCATION`     | `location-always`      |
| `contacts`       | `READ_CONTACTS`                                           | `contacts`             |
| `calendar`       | `READ_CALENDAR`                                           | `calendar`             |
| `photos`         | SDK-conditional: `READ_MEDIA_IMAGES` (+ `…_VISUAL_USER_SELECTED` on Android 14+) / `READ_EXTERNAL_STORAGE` on Android 12 and below | `photos` |

Unknown shorthands fail loudly at the action layer (no silent passthrough to `pm grant` / `simctl privacy`). You can also pass a full `android.permission.*` string and Android will use it verbatim.

> **Note: notifications aren't a pre-grantable shorthand.** Both iOS and Android (13+) show a system dialog the first time the app calls the notification-authorization API — pre-granting is Android-only and breaks parity. The cross-platform pattern is to trigger the request from inside the app and dismiss the dialog with `accept_alert`:
>
> ```toml
> { action = "tap", on_text = "Enable Notifications" }
> { action = "accept_alert", if_fail = "ignore" }
> ```
>
> `if_fail = "ignore"` keeps the step happy on warm sims/emulators that have already recorded the user's prior choice and skipped the prompt.

The test app's `AndroidManifest.xml` must declare every permission you intend to `grant_permission` — `pm grant` rejects undeclared permissions. The committed manifest in `test-app/src-tauri/gen/android/app/src/main/AndroidManifest.xml` declares all the shorthands above; copy that pattern for your own apps.

## Capture

### `screenshot` — Take screenshot

```toml
{ action = "screenshot" }
{ action = "screenshot", path = "/tmp/dark-mode.png" }
```

### Screen recording — per-block via `record = true`

Recording is configured at the project, flow, or block level — not as a
step action. Cascade (highest priority wins): `--no-record` >
`--record` > `[[block]] record` > `[flow.options] record` >
`[options] record`. Output: `{output_dir}/{flow}/{device}/recordings/{block}_{iter}.mp4`.

```toml
[[block]]
name = "login"
record = true     # record this block only
steps = [ ... ]
```

### `add_media` — Push media to device

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

## Alerts

### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes) on the current alert.

```toml
{ action = "accept_alert" }
```

### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No) on the current alert.

```toml
{ action = "dismiss_alert" }
```

## External

### `open_link` — Open URL or deep link

```toml
{ action = "open_link", url = "https://example.com" }
{ action = "open_link", url = "myapp://profile/123" }
```

### `push_notification` — Deliver a push to the app under test

```toml
{ action = "push_notification", title = "New message", body = "Hello!", app = "app" }
```

The action injects a push payload via the platform's developer backdoor — `xcrun simctl push` on iOS, `adb shell am broadcast` on Android — so the app's notification receiver fires in foreground. The app's own receive bridge (UNUserNotificationCenterDelegate on iOS, BroadcastReceiver on Android) handles the payload; the action exercises that bridge end-to-end without requiring real APNS / FCM infrastructure.

| Field | Description |
|-------|-------------|
| `app` | App registry name from `golem.toml` (required — resolves the bundle id) |
| `title` | Notification title (whitespace and quotes safe on both platforms) |
| `body` | Notification body |
| `payload` | Optional structured payload — merged into the APNS dict as `custom` on iOS; ignored on Android |

**Sim/emu only on both platforms.** Physical-device push delivery needs real APNS / FCM (provisioning keys, device tokens, network) which is outside this action's scope. On a physical device the driver bails with a clear error pointing at this paragraph.

Compose physical-device push tests by branching on `_hardware` and posting to your own backend via `*_http`:

```toml
[[block]]
name = "trigger_push_virtual"
[[block.branch]]
if_var = "_hardware"
equals = "virtual"
goto = "send_via_simctl"
[[block.branch]]
if_var = "_hardware"
equals = "real"
goto = "send_via_backend"

[[block]]
name = "send_via_simctl"
steps = [
  { action = "push_notification", title = "Test", body = "Hello", app = "app" },
]

[[block]]
name = "send_via_backend"
steps = [
  { action = "post_http", url = "https://your-test-backend/push", body = "{\"device\":\"${device.udid}\",\"body\":\"Hello\"}" },
]
```

A `[lint]` warning fires at parse time when a flow uses `push_notification` and any of its apps declares `hardware = "real"` or `["virtual", "real"]`, so authors get an early breadcrumb that the action will fail on the phys branch unless they wrap it in `branch` like above.

**Receive bridge.** The action only delivers — the app must wire up its native receiver to forward the payload into its UI / state. See `test-app-b/ios/GolemTestB/GolemTestBApp.swift` and `test-app-b/android/app/src/main/java/fail/golem/testb/MainActivity.kt` for a minimal SwiftUI / Compose implementation. Tauri 2.x's `@tauri-apps/plugin-notification` is for *local* notifications (app schedules its own); it doesn't expose remote-push delivery to JS today, which is why `test-app` (Tauri) doesn't carry the bridge and `test-app-b` (native) does.

### `bash` — Run shell command

Execute a command via `sh -c`. Fails if exit code is non-zero.

```toml
{ action = "bash", run = "curl -s https://api.example.com/reset" }
{ action = "bash", run = "echo $ENV_VAR", save_to = "result" }
```

### `run` — Run project script

Execute a script relative to the project root or flow directory. Rejects path traversal (`..`).

```toml
{ action = "run", script = "/scripts/seed_db.sh" }
{ action = "run", script = "/scripts/setup.sh", args = ["staging", "verbose"], save_to = "output" }
```

Leading `/` = relative to project root. No leading `/` = relative to flow file directory.

### `create_inbox` — Provision a disposable email inbox

Provision a fresh inbox from a provider and save its connection details as an
object for later steps. The saved object's `imap_host`/`imap_port`/`user`/`pass`
fields are exactly what [`await_email`](#await_email--poll-imap-inbox) reads, so
`save_to = "inbox"` feeds straight into `await_email { inbox = "inbox" }`.

```toml
{ action = "create_inbox", provider = "ethereal", save_to = "inbox" }
{ action = "type", on_text = "Email", input = "${inbox.address}" }
# … app signup …
{ action = "await_email", inbox = "inbox", subject = "*verify*", extract = { otp = "code: (\\d{6})" }, save_to = "mail" }
{ action = "type", input = "${mail.otp}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `provider` | — | Inbox provider. Only `ethereal` is built in; any other value errors. |
| `save_to` | — | Variable to store the inbox object under (required). |
| `timeout` | `15000` | Provisioning deadline (ms). |

Saved object fields: `address` (= `user`, the email address), `user`, `pass`,
`imap_host`, `imap_port`, `smtp_host`, `smtp_port`.

> **Non-deterministic.** Provisioning is live network I/O, so the inbox is not
> replayed by `--seed` — each run gets a fresh address. Receiving mail at it is
> live too (`await_email` connects over real IMAP).

### `await_email` — Poll IMAP inbox

Poll an inbox over IMAP (TLS) and wait for an email matching the filters, with
optional regex extraction.

`inbox` is **not** the email address — it is the **name of a variable** holding
an inbox object (the one [`create_inbox`](#create_inbox--provision-a-disposable-email-inbox)
saved, or a `[flow.vars]` table you wrote). The action reads four fields from
that object by name: `imap_host`, `imap_port`, `user`, `pass`. So
`create_inbox { save_to = "inbox" }` pairs with `await_email { inbox = "inbox" }`.

```toml
# Pairs with create_inbox { save_to = "inbox" }:
{ action = "await_email", inbox = "inbox", subject = "Verify*", timeout = 30000, save_to = "email" }

# …or a hand-written inbox object:
# [flow.vars]
# inbox = { imap_host = "imap.example.com", imap_port = "993", user = "me@example.com", pass = "secret" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `inbox` | — | Name of a variable holding an inbox object; the `imap_host` / `imap_port` / `user` / `pass` fields on it are used to connect |
| `to` | — | Glob filter for recipient |
| `subject` | `"*"` | Subject glob pattern |
| `extract` | — | Table of field names to regex patterns |
| `timeout` | `30000` | Polling timeout (ms) |

When more than one email matches, the **most recent** is returned, so a stale
match left in the inbox from an earlier run never shadows the fresh one. Only
the latest messages are scanned (not the entire mailbox), which is ample for
verification/OTP mail.

### `load_fixture` — Load fixture data

Load variables from a TOML file in `__fixtures__/`.

```toml
{ action = "load_fixture", fixture = "users", as = "test_user" }
# Access as ${test_user.email}, ${test_user.name}, etc.
```

### `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests

```toml
{ action = "get_http", url = "https://api.example.com/status", save_to = "response" }
{ action = "post_http", url = "https://api.example.com/reset", body = "{\"force\": true}" }
{ action = "get_http", url = "https://api.example.com/data", headers = { Authorization = "Bearer ${token}" } }
```

Fails on non-2xx status codes.

## Flow Control

### `fail` — Fail the flow immediately

```toml
{ action = "fail", message = "Unexpected state reached" }
{ action = "fail", message = "Bad total: ${order.total}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `message` | `"Flow failed (no message provided)"` | Failure reason shown in reports; supports inline `${…}` vars |

The only field `fail` uses is `message`. Useful in conditional branches to mark
unreachable paths.
