<!-- Generated from docs/src/actions-reference/ — edit the parts there, then run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs`. -->
# Actions Reference

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) for selectors, steps, and flow anatomy.

## Contents

- [Interaction](#interaction)
  - [`tap`](#tap--tap-an-element)
  - [`double_tap`](#double_tap--double-tap-an-element)
  - [`type`](#type--type-text-into-an-element)
  - [`backspace`](#backspace--delete-characters)
  - [`clear_text`](#clear_text--empty-a-field)
  - [`long_press`](#long_press--long-press-an-element)
  - [`swipe`](#swipe--swipe-gesture)
  - [`scroll`](#scroll--scroll-until-element-found)
  - [`pinch`](#pinch--pinch-zoom-gesture)
  - [`gesture`](#gesture--multi-touch-gesture)
  - [`rotate`](#rotate--rotate-gesture)
  - [`hide_keyboard`](#hide_keyboard--dismiss-keyboard)
- [Assertions](#assertions)
  - [`assert_visible`](#assert_visible--wait-for--assert-element-exists)
  - [`assert_not_visible`](#assert_not_visible--wait-for--assert-element-absent)
  - [`assert_alert`](#assert_alert--assert-alert-is-displayed)
- [Reading](#reading)
  - [`read`](#read--read-element-text)
- [App Lifecycle](#app-lifecycle)
  - [`launch`](#launch--launch-or-foreground-an-app)
  - [App permissions](#app-permissions)
  - [`stop`](#stop--terminate-an-app)
  - [`clear_data`](#clear_data--clear-app-data)
- [Device Controls](#device-controls)
  - [`set_dark_mode`](#set_dark_mode--set-dark-mode)
  - [`set_location`](#set_location--set-gps-coordinates)
  - [`press`](#press--press-hardware-button)
- [Capture](#capture)
  - [`screenshot`](#screenshot--take-screenshot)
  - [Screen recording](#screen-recording--per-block-via-record--true)
  - [`add_media`](#add_media--push-media-to-device)
- [Alerts](#alerts)
  - [`accept_alert`](#accept_alert--accept-dialog)
  - [`dismiss_alert`](#dismiss_alert--dismiss-dialog)
- [External](#external)
  - [`open_link`](#open_link--open-url-or-deep-link)
  - [`push_notification`](#push_notification--deliver-a-push-to-the-app-under-test)
  - [`bash`](#bash--run-shell-command)
  - [`run`](#run--run-project-script)
  - [`create_inbox`](#create_inbox--provision-a-disposable-email-inbox)
  - [`await_email`](#await_email--poll-imap-inbox)
  - [`load_fixture`](#load_fixture--load-fixture-data)
  - [`load_mixin`](#load_mixin--inline-a-reusable-step-sequence)
  - [`get_http`, `post_http`, `put_http`, `patch_http`, `delete_http`](#get_http-post_http-put_http-patch_http-delete_http--http-requests)
- [Browser](#browser)
  - [`browse_navigate`](#browse_navigate--load-a-url)
  - [`browse_tap`](#browse_tap--click-an-element)
  - [`browse_type`](#browse_type--type-into-a-field)
  - [`browse_read`](#browse_read--read-text-or-an-attribute-into-a-variable)
  - [`browse_screenshot`](#browse_screenshot--capture-the-tab)
  - [`browse_assert_exists`](#browse_assert_exists--the-element-is-in-the-dom)
  - [`browse_assert_not_exists`](#browse_assert_not_exists--the-element-is-not-in-the-dom)
  - [`browse_assert_text`](#browse_assert_text--the-element-says-what-you-expect)
  - [`browse_wait_exists`](#browse_wait_exists--wait-for-an-element-to-appear)
  - [`browse_wait_not_exists`](#browse_wait_not_exists--wait-for-an-element-to-disappear)
  - [`browse_scroll_by`](#browse_scroll_by--scroll-by-a-distance)
  - [`browse_scroll_to`](#browse_scroll_to--bring-an-element-into-view)
  - [`browse_select`](#browse_select--choose-an-option-in-a-select)
  - [`browse_execute_js`](#browse_execute_js--run-javascript-in-the-page)
  - [`browse_mcp_list_tools`](#browse_mcp_list_tools--list-the-pages-webmcp-tools)
  - [`browse_mcp_call`](#browse_mcp_call--call-a-webmcp-tool)
  - [`browse_set_cookie` / `browse_get_cookie`](#browse_set_cookie--browse_get_cookie--cookies)
  - [`browse_set_local_storage` / `browse_get_local_storage`](#browse_set_local_storage--browse_get_local_storage--local-storage)
  - [`browse_set_session_storage` / `browse_get_session_storage`](#browse_set_session_storage--browse_get_session_storage--session-storage)
  - [`browse_close`](#browse_close--close-a-tab-early)
- [Flow Control](#flow-control)
  - [`fail`](#fail--fail-the-flow-immediately)

<!-- The canonical list of action keywords is the dispatch match in [`golem-runner/src/actions.rs`](../golem-runner/src/actions.rs), plus [`golem-browser/src/actions.rs`](../golem-browser/src/actions.rs) for the `browse_*` family. If you add a handler in either, document it here — `actions_reference_doc_lists_every_action` enforces it. -->

## Interaction

Touch and keyboard input on an element.

### `tap` — Tap an element

Find an element matching the selectors and tap its center.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "tap", on_text = "+", timeout = 5000 }
{ action = "tap", on = { text = "OK", below = "Confirm?" } }
{ action = "tap", on_accessibility_label = "Increment" }
```

On iOS a tap holds for about 50 ms. If the app reads that as a long press,
use [`long_press`](#long_press--long-press-an-element) instead.

### `double_tap` — Double-tap an element

Two rapid taps at the element center.

```toml
{ action = "double_tap", on_text = "Zoom" }
```

Same selectors and options as `tap`.

### `type` — Type text into an element

With a selector, taps the element to focus it, then types the `input`
string. No selector = type into the focused field.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
{ action = "type", input = " and more" }   # append to the focused field
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |

### `backspace` — Delete characters

Deletes `count` characters before the caret in the **currently focused** text
field. `type` or `tap` the field first; `type` leaves the caret at the end of
the text. A selector is an error.

```toml
{ action = "type", on_text = "Email", input = "me@example.comm" },
{ action = "backspace", count = 1 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `count` | `1` | Number of characters to delete from the focused field |

### `clear_text` — Empty a field

Empties the **currently focused** text field in one step, whatever its length.
Like `backspace` it takes **no selector** — `type` or `tap` the field first.
Use it when the field's contents aren't known up front (a value carried over
from a previous run, a prefilled form); reach for `backspace` when you want to
remove a specific number of characters.

```toml
{ action = "tap", on_text = "Email" },
{ action = "clear_text" },
{ action = "type", input = "me@example.com" }
```

Takes no fields.

- **The caret must be at the end.** If it isn't, the step fails and tells you
  to re-focus the field. `type` leaves the caret at the end; a `tap` places it
  where you tapped.
- **A field whose contents exactly equal its placeholder reads as empty** and is
  left alone.

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
{ action = "swipe", start = { text = "Slider" }, end = { text = "Max" } }

# Anchored to a container: end is 30% of the element's height below its centre
{ action = "swipe",
  start = { below = "Scroll List" },
  end   = { below = "Scroll List", y = "30%" } }
```

| Field | Description |
|-------|-------------|
| `direction` | `"up"`, `"down"`, `"left"`, `"right"` |
| `start` | Start position: a selector group (`text` / `accessibility_label` / `below` / `above`) plus optional `x` / `y`. With an element, `x` / `y` offset from its centre: pixels, or `"N%"` of the element's size (`"50%"` = edge). Without an element, `x` / `y` are screen pixels or `"N%"` of the screen. |
| `end` | End position, same format as `start` |
| `points` | Array of intermediate points, same format as `start` |
| `duration` | Gesture duration in ms for a path of 3+ points (default `300`); a 2-point swipe ignores it |

`within` is ignored on swipe (lint warning); use `start` / `end`.

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
| `to` | — | Target element: a selector group (alias of `on`), or use the flat `on_*` selectors |
| `direction` | `"down"` | Scroll direction |
| `within` | — | Constrain scrolling to an element's bounds |
| `max_scrolls` | — | Limit iterations |
| `timeout` | 8× `step_timeout` (40 s); 12× (60 s) with `within` | Overall scroll timeout |

### `pinch` — Pinch zoom gesture

Two-finger pinch centered on an element or coordinates.

```toml
{ action = "pinch", scale = 2.0, duration = 500 }     # Zoom in
{ action = "pinch", scale = 0.5, duration = 500 }     # Zoom out
{ action = "pinch", on_text = "Map", scale = 2.0 }    # Centered on an element
{ action = "pinch", x = "50%", y = 300, scale = 0.5 } # Centered on a point
```

| Field | Default | Description |
|-------|---------|-------------|
| `scale` | — | `>1.0` = zoom in, `<1.0` = zoom out |
| `velocity` | `5.0` | Scale factor per second |
| center | screen centre | An element (`on_text`, `on_accessibility_label` or `on = { … }`), or `x` / `y` as pixels or `"N%"` of the screen. With an element, `x` / `y` are ignored. |

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
| `fingers` | — | Array of finger paths, each with `points` (at least 2 per finger) |
| `duration` | `300` | Time (ms) each finger takes to travel its whole path |

A point is `x` / `y` screen coordinates (pixels or `"N%"` of the screen), or a
selector group (`text` / `accessibility_label` / `below` / `above`) plus
optional `x` / `y` offsets from the element's centre (pixels, or `"N%"` of the
element's size).

### `rotate` — Rotate gesture

A two-finger **rotation gesture** centered on an element (or screen). `rotate` is a multi-touch gesture, **not** a device-orientation change — programmatic device orientation is [unsupported](unsupported.md).

Two fingers orbit a center point: an element (`on_text`, `on_accessibility_label` or `on = { … }`), or `x` / `y` as pixels or `"N%"` of the screen. Without either, the screen centre. With an element, `x` / `y` are ignored.

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

You rarely need it to reach a field the keyboard covers: when the keyboard
hides a step's target, golem dismisses the keyboard and looks again. Set
`keep_keyboard = true` on a step to opt out (see
[Step Options](test-structure.md#step-options)):

```toml
{ action = "tap", on_text = "Done", keep_keyboard = true }
```

## Assertions

Check what the screen shows. An assertion waits up to its timeout for the screen to match.

### `assert_visible` — Wait for / assert element exists

Poll the hierarchy until an element matching the selectors is on screen, or `timeout` elapses (default 10s). Use a short `timeout` for instantaneous checks, a long one for waits. The assertion is driven by the selectors — add `on_enabled` / `on_checked` to assert state, not just presence.

```toml
{ action = "assert_visible", on_text = "Welcome" }
{ action = "assert_visible", on_text = "1", on_below = "Counter" }
{ action = "assert_visible", on_text = "Submit", on_enabled = true }                    # state, not just presence
{ action = "assert_visible", on_text = "Item 0", auto_scroll = true, timeout = 60000 }  # off-screen element
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

Read an element's text into a variable.

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

Start, stop and reset an app, and set its permissions.

### `launch` — Launch or foreground an app

Bring an app to the foreground. Does not restart if already running. Use `restart = true` for a cold start.

```toml
{ action = "launch", app = "app" }
{ action = "launch", app = "companion" }
{ action = "launch", app = "app", restart = true }                        # Kill and relaunch
{ action = "launch", app = "app", permissions = { camera = "allow" } }    # Grant, then cold start
```

| Field | Default | Description |
|-------|---------|-------------|
| `app` | — | App name (as defined in `[[flow.apps]]`) |
| `restart` | `false` | Stop app first, then launch fresh |
| `permissions` | — | Per-launch permission map (see [App permissions](#app-permissions)) |

A `permissions` map always cold-starts the app, even without `restart = true`. Use it to flip a permission mid-flow and test both the granted and denied paths in one flow.

### App permissions

Permissions are set **on a launch**, not as a standalone step. Two places take the same `permission = mode` map:

- **`[[flow.apps]].permissions`** — the baseline, applied once before the app's first launch (see [Test structure](test-structure.md#launch-time-permissions)).
- **`launch` action `permissions =`** — a per-launch override for changing a permission later in the flow. It always cold-starts the app.

```toml
{ action = "launch", app = "app", permissions = { camera = "allow" } }
# ... exercise the granted path ...
{ action = "launch", app = "app", permissions = { camera = "deny" } }
# ... exercise the denied path ...
```

**Modes.** The value is a mode, not just on/off:

| Mode | Applies to | Meaning |
|------|-----------|---------|
| `allow` / `deny` | any permission | grant / explicitly denied (not reset to "not asked"). For `location`, `allow` grants foreground ("when in use"). |
| `always` | `location` only | grant background + foreground location |
| `limited` | `photos` only | partial photo-library access (iOS limited library; Android 14+ user-selected subset) |

An invalid mode for a permission (e.g. `camera = "limited"`) is a parse-time error.

**Permissions.** One vocabulary for both platforms: `camera`, `microphone`, `location`, `contacts`, `calendar`, `photos`. An unknown name is an error. On Android you can also pass a full `android.permission.*` string.

Your app's `AndroidManifest.xml` must declare every permission you grant. For `photos`, declare `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED` and `READ_EXTERNAL_STORAGE`: golem grants a different one per Android version. `location = "always"` also needs `ACCESS_BACKGROUND_LOCATION`.

> **iOS `photos` needs [`applesimutils`](https://github.com/wix/AppleSimulatorUtils)** (`brew tap wix/brew && brew install wix/brew/applesimutils`) to grant without a prompt; `golem doctor` flags it if missing. Without it golem warns and the app prompts at runtime, so add `{ action = "accept_alert", if_fail = "ignore" }` after the step that triggers photo access.

> **Notifications can't be pre-granted.** Both platforms show a system dialog the first time the app asks. Trigger the request from the app and accept the dialog:
>
> ```toml
> { action = "tap", on_text = "Enable Notifications" }
> { action = "accept_alert", if_fail = "ignore" }
> ```
>
> `if_fail = "ignore"` covers a device that already recorded a choice and skips the prompt.

### `stop` — Terminate an app

Terminate the app's process. The next `launch` starts it fresh.

```toml
{ action = "stop", app = "app" }
```

### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

iOS: simulator only; gate on `_hardware`. Android works on emulators and physical devices.

To reset state on a physical iOS device, either drive the app's own "sign out" / "reset" affordance, or reinstall it (the install script runs before every flow; `GOLEM_REBUILD` forces a fresh build). Gate the step on device class if one flow must cover both:

```toml
[[block.branch]]
if_var = "_hardware"
equals = "virtual"
goto = "wipe_via_clear_data"
[[block.branch]]
if_var = "_hardware"
equals = "real"
goto = "wipe_via_app_ui"
```

## Device Controls

Change device settings: dark mode, location, hardware buttons.

### `set_dark_mode` — Set dark mode

Switch the device's system appearance to dark (`enabled = true`) or light (`enabled = false`).

```toml
{ action = "set_dark_mode", enabled = true }
{ action = "set_dark_mode", enabled = false }
```

iOS: simulator only; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch).

### `set_location` — Set GPS coordinates

Set the device's GPS position to `latitude` / `longitude` (decimal degrees).

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

iOS: simulator only; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch).

### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

**Supported buttons (platform-specific):**

| `button`      | Android | iOS |
|---------------|---------|-----|
| `home`        | ✓       | ✓   |
| `back`        | ✓       | —   |
| `volume_up`   | ✓       | —   |
| `volume_down` | ✓       | —   |

An unsupported button fails the step.

## Capture

Screenshots, screen recordings, and media files pushed to the device.

### `screenshot` — Take screenshot

Capture the screen. With `path`, save it there; a relative path resolves from the directory where you run golem. Without `path`, the image is captured but not saved.

```toml
{ action = "screenshot" }
{ action = "screenshot", path = "/tmp/dark-mode.png" }
```

### Screen recording — per-block via `record = true`

Recording is configured with `record`, not as a step action (see [Flow Options](test-structure.md#flow-options)). Highest priority wins: `--no-record` > `--record` > `[[block]] record` > `[flow.options] record` > `[options] record`. Output: `{output_dir}/{flow}/{device}/recordings/{block}_{iter}.mp4`.

```toml
[[block]]
name = "login"
record = true     # record this block only
steps = [ ... ]
```

iOS: simulator only. On a physical iPhone the block logs a warning and runs without a video.

### `add_media` — Push media to device

Add an image or video file to the device's photo library. golem checks the file's contents: anything other than an image (png, jpeg, gif, webp, heif, bmp, tiff) or a video (mp4, mov) fails with P462. A relative `path` resolves from the directory where you run golem.

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

iOS: simulator only; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch), or put the file in the device's library before the run.

## Alerts

Accept or dismiss a dialog from the app or the OS.

### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes, Allow) on the current alert. It also handles OS prompts, such as permission requests and "Open in …?" dialogs.

```toml
{ action = "accept_alert" }
{ action = "accept_alert", if_fail = "ignore" }   # prompt may not appear
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a prompt that may not appear.

### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No) on the current in-app alert. It may not reach an OS prompt (an iOS permission dialog, for example); `accept_alert` handles those.

```toml
{ action = "dismiss_alert" }
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a dialog that may not appear.

## External

Work outside the screen: links, pushes, shell commands, email, fixtures, mixins and HTTP.

### `open_link` — Open URL or deep link

Open a URL or deep link on the device.

```toml
{ action = "open_link", url = "https://example.com" }
{ action = "open_link", url = "myapp://profile/123" }
```

### `push_notification` — Deliver a push to the app under test

Deliver a push notification to the app under test without APNs or FCM. The app's own receive bridge must forward the payload into its UI or state.

```toml
{ action = "push_notification", title = "New message", body = "Hello!" }
```

| Field | Description |
|-------|-------------|
| `title` | Notification title |
| `body` | Notification body |
| `payload` | Optional. iOS only: a string holding a JSON object, added to the APNs payload as `custom`. Ignored on Android. |
| `app` | Optional. Delivery always goes to the app under test; `app` only tells the lint below which app's `hardware` to check. |

**Simulator and emulator only, on both platforms**; gate on `_hardware` (see [`clear_data`](#clear_data--clear-app-data) for the branch). On real hardware, send the push from your own backend with `post_http` in the `real` branch.

A flow whose app may run on real hardware (`hardware` unset or including `"real"`) gets a `[lint]` warning; set `hardware = "virtual"` to silence it.

**Receive bridge.** The app must handle the delivered push: a `UNUserNotificationCenterDelegate` on iOS, and on Android a `BroadcastReceiver` for the action `<package>.PUSH_NOTIFICATION` that reads the `title` and `body` string extras. See `test-app-b/ios/GolemTestB/GolemTestBApp.swift` and `test-app-b/android/app/src/main/java/fail/golem/testb/MainActivity.kt` for a minimal implementation.

### `bash` — Run shell command

Execute a command via `sh -c`. A non-zero exit code fails the step and reports the command's stderr. `save_to` stores stdout, trimmed.

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

The script runs directly, not through a shell, so it must be executable (`chmod +x`) and start with a shebang. A non-zero exit code fails the step and reports stderr. `save_to` stores an object: `${output.stdout}` (trimmed) and `${output.exit_code}`.

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

`inbox` is the **name of a variable** holding an inbox object, not the email
address. See [`create_inbox`](#create_inbox--provision-a-disposable-email-inbox)
for how the two pair up.

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
| `recipient` | — | Glob filter for the recipient address (not `to`) |
| `subject` | `"*"` | Subject glob pattern |
| `extract` | — | Table of field names to regex patterns |
| `timeout` | `30000` | Polling timeout (ms) |

When more than one email matches, the **most recent** is returned, so a stale
match left in the inbox from an earlier run never shadows the fresh one. Only
the latest messages are scanned (not the entire mailbox), which is ample for
verification/OTP mail.

### `load_fixture` — Load fixture data

Load variables from a TOML file in `__fixtures__/` (a `[vars]` table). See
[reuse comparison](test-structure.md#reuse-subflow-vs-mixin-vs-fixture).

```toml
{ action = "load_fixture", fixture = "users", as = "test_user" }
# Access as ${test_user.email}, ${test_user.name}, etc.
```

| Field | Description |
|-------|-------------|
| `fixture` | Fixture name: `__fixtures__/<fixture>.toml`, looked up from the flow's directory up to the project root (required) |
| `as` | Variable name the fixture's vars are stored under (required) |

### `load_mixin` — Inline a reusable step sequence

Inline the steps from a mixin file in `__mixins__/` (a `[[step]]`-only file) into
the current block. Pass per-call values via `vars`, referenced as `${…}` inside
the mixin. See [reuse comparison](test-structure.md#reuse-subflow-vs-mixin-vs-fixture).

```toml
# __mixins__/launch_and_wait.toml
[[step]]
action = "launch"
app = "${app_bundle}"
[[step]]
action = "assert_visible"
on_text = "${wait_element}"
```

```toml
{ action = "load_mixin", mixin = "launch_and_wait", vars = { app_bundle = "app", wait_element = "Submit" } }
```

A mixin is a step fragment that runs inside the caller's block; for reusing a
whole scenario as a child, use a [subflow](test-structure.md#subflow) instead.

### `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests

```toml
{ action = "get_http", url = "https://api.example.com/status", save_to = "response" }
{ action = "post_http", url = "https://api.example.com/reset", body = "{\"force\": true}" }
{ action = "get_http", url = "https://api.example.com/data", headers = { Authorization = "Bearer ${token}" } }
```

| Field | Description |
|-------|-------------|
| `url` | Request URL (required) |
| `body` | Request body, as a string |
| `headers` | Table of header name to string value |
| `save_to` | Variable to store the response body under, as a string |

A non-2xx status fails the step.

## Browser

Drive a host browser, for web state that the app depends on.

Host-side browser automation, for flows whose mobile app depends on web state
nothing else can reach — a supplier fulfilling an order through a portal with no
API, an admin console that flips a feature flag.

**The browser is instrumentation, not the system under test.** The mobile app is
what golem tests, so browser steps are not judged for coverage and never feed
the accessibility audit. They check **DOM presence, not visibility**: an element
hidden by CSS still counts as present, unlike the mobile
[visible tree](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints).

Targeting is **CSS only**. golem's mobile selectors (`text`, `on_below`, and the
rest) describe a native view tree and are ignored by a browser step.

| Field | Default | Description |
|-------|---------|-------------|
| `selector` | — | CSS selector, passed to the page verbatim. Required by the actions that act on an element: `browse_tap`, `browse_type`, `browse_read`, `browse_select`, `browse_scroll_to`, and the `browse_assert_*` and `browse_wait_*` actions |
| `index` | `0` | Which match to act on when the selector matches several, 0-based — same numbering as [`on_index`](selectors.md) |
| `session` | `_default` | `[context:]tab`. Tabs share a context's cookies, so a login carries between them; separate contexts share nothing |
| `timeout` | `5000`; `10000` for `browse_wait_exists` and `browse_wait_not_exists` | How long to keep looking for the element, in ms |

**Requires a Chrome or Chromium on the host**, or `$CHROME` pointing at one;
golem never downloads one. A browser flow on a machine without one fails at plan
time with `H424`, before any device boots. A suite with no `browse_*` step never
looks for one.
Each flow gets its own browser, so concurrent flows never share cookies or
storage, and it is closed when the flow ends whether it passed or failed.

Storage and cookies need a real origin: a page reached by `browse_navigate` has
one, but `about:blank` doesn't, and storage and cookie steps fail there.

**Tabs and contexts.** `session = "admin"` opens a named tab. `session =
"tenantB:admin"` opens that tab in a separate **context** — its own cookie jar —
which is what the same site logged in as two different users at once requires,
since tabs share a login:

```toml
{ action = "browse_navigate", url = "${portal}", session = "tenantA:main" }
{ action = "browse_navigate", url = "${portal}", session = "tenantB:main" }
# tenantA and tenantB can now hold different sessions on the same domain
```

Contexts are created on first use and closed with the flow. `:` is the
separator, so neither label may contain one. There is no sticky context: a step
without the prefix uses the flow's default one.

### `browse_navigate` — Load a URL

```toml
{ action = "browse_navigate", url = "https://portal.example.com/orders" }
{ action = "browse_navigate", url = "${portal_url}", wait_until = "load" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `url` | — | Required |
| `wait_until` | `"domcontentloaded"` | When the step returns: `none` (as soon as Chrome accepts it), `domcontentloaded` (the DOM is parsed and queryable), `load` (sub-resources finished too) |
| `user_agent` | the browser's own | Present this session as a different client. Applied **before** the navigation, so the first request carries it |
| `accept_language` | the browser's own | `Accept-Language` for this session, e.g. `"fr-FR"` |

**Identity belongs to the tab, not the step.** `user_agent` and
`accept_language` stay in force for that `session` until something changes
them, so set them on the session's first navigation and later steps inherit
them — a session that is a phone stays a phone. Two sessions can hold different
identities at once:

```toml
{ action = "browse_navigate", url = "${portal}", session = "phone",
  user_agent = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) …" }
{ action = "browse_navigate", url = "${portal}", session = "desktop" }
```

Two things this does **not** do. It doesn't change rendering — layout follows
the viewport, so a page won't reflow to phone width because its user agent says
iPhone. And it doesn't touch [Client Hints](https://developer.chrome.com/docs/privacy-security/user-agent-client-hints):
`Sec-CH-UA-Mobile` and friends still describe the real Chrome, so a portal that
reads those rather than the user-agent string will see through it.

### `browse_tap` — Click an element

```toml
{ action = "browse_tap", selector = "#submit" }
{ action = "browse_tap", selector = "button.fulfil", index = 1 }
```

A real click, so the page's own handlers run.

### `browse_type` — Type into a field

```toml
{ action = "browse_type", selector = "#email", text = "${inbox.address}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | — | The value to type. `input` is accepted as a synonym, matching the mobile `type` action |

The field is clicked first, so the keystrokes land in it rather than wherever
focus happened to be.

### `browse_read` — Read text or an attribute into a variable

```toml
{ action = "browse_read", selector = "#order-total", save_to = "total" }
{ action = "browse_read", selector = "#order-total", attribute = "data-total", save_to = "total_raw" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `attribute` | — | Read this attribute instead of the element's text, e.g. `data-total="1499"` where the text says `£14.99` |
| `save_to` | — | Variable to save the value in |

Fails with `F404` if the element has no such attribute, rather than saving an empty string.

### `browse_screenshot` — Capture the tab

```toml
{ action = "browse_screenshot", path = "portal-state.png" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `path` | — | Where to write the PNG. Omitted, the capture is taken and discarded — same as the mobile `screenshot` action |

### `browse_assert_exists` — The element is in the DOM

```toml
{ action = "browse_assert_exists", selector = ".order-row[data-state='fulfilled']" }
```

Waits up to `timeout` for the element to appear. Fails with `F404` if it never does.

### `browse_assert_not_exists` — The element is not in the DOM

```toml
{ action = "browse_assert_not_exists", selector = "#error-banner" }
```

Checks once and fails with `F409` if the element is there. It does **not** wait
for something to disappear — that's `browse_wait_not_exists`.

### `browse_assert_text` — The element says what you expect

```toml
{ action = "browse_assert_text", selector = "#status", text = "Fulfilled" }
{ action = "browse_assert_text", selector = "#total", text = "Total: *" }
{ action = "browse_assert_text", selector = "#total", attribute = "data-total", text = "1499" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | — | Expected value. Supports the same `*` / `?` [glob matching](selectors.md) as mobile text matchers, and is case-sensitive |
| `attribute` | — | Assert on this attribute instead of the element's text |

Polls until the text matches or `timeout` runs out, so a page that updates after
a click isn't judged on what it said beforehand. The failure (`F412`) quotes what
the page actually said.

### `browse_wait_exists` — Wait for an element to appear

```toml
{ action = "browse_wait_exists", selector = ".order-row" }
{ action = "browse_wait_exists", selector = "#receipt", timeout = 30000 }
```

Polls until the element is in the DOM. Default `timeout` is 10000ms. Running out
fails with `F408` (step timeout), not `F404`.

### `browse_wait_not_exists` — Wait for an element to disappear

```toml
{ action = "browse_wait_not_exists", selector = ".spinner" }
```

Polls until the element is gone from the DOM. Default `timeout` is 10000ms.
Running out fails with `F408`. To check absence once without waiting, use
`browse_assert_not_exists`.

### `browse_scroll_by` — Scroll by a distance

```toml
{ action = "browse_scroll_by" }                                  # 300px down, the page
{ action = "browse_scroll_by", direction = "up", amount = 800 }
{ action = "browse_scroll_by", container = ".results", amount = 500 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `direction` | `"down"` | `up`, `down`, `left` or `right` — same values and default as the mobile `swipe`/`scroll` actions |
| `amount` | `300` | Distance in CSS pixels |
| `container` | — | Scroll this element instead of the window |

### `browse_scroll_to` — Bring an element into view

```toml
{ action = "browse_scroll_to", selector = "#order-footer" }
```

Useful before a `browse_screenshot`, or for a page that only renders on scroll.

### `browse_select` — Choose an option in a `<select>`

```toml
{ action = "browse_select", selector = "#status", value = "fulfilled" }
{ action = "browse_select", selector = "#status", text = "Fulfilled" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `value` | — | Match the option's `value` attribute |
| `text` | — | Match the option's visible label instead. Give one or the other, not both |

Fires `input` and `change` the way a user's choice would, so a framework
listening for them sees the update.

### `browse_execute_js` — Run JavaScript in the page

```toml
{ action = "browse_execute_js", script = "return document.title", save_to = "title" }
{ action = "browse_execute_js", file = "portal-helpers.js", script = "return fulfilOrder('${order_id}')" }
{ action = "browse_execute_js", script = "const r = await fetch('/api/orders'); return (await r.json()).length", save_to = "count" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `script` | — | Inline JavaScript. Golem `${…}` variables are interpolated here |
| `file` | — | A `.js` file to run first. A leading `/` resolves from the project root; anything else from the flow file's directory. `..` is rejected |
| `save_to` | — | Save the result. Objects nest, so `${result.total}` works; anything else is stored as text |

Give either, or both. **The file runs first** — it's the natural home for
reusable functions, and the inline script is then the one-liner that calls one.
They run as a single evaluation, so the inline script sees whatever the file
declared.

The script body runs inside an async function: `return` what you want to save,
and `await` is available for anything the page has to fetch.

Golem variables are interpolated into `script` but **not** into `file`. Both are
JavaScript, not TypeScript.

### `browse_mcp_list_tools` — List the page's WebMCP tools

```toml
{ action = "browse_mcp_list_tools", save_to = "tools" }
```

A page that opts into [WebMCP](https://developer.chrome.com/docs/ai/webmcp)
describes what it can do — "fulfil an order", "issue a refund" — as named tools
with argument schemas. Saved as an object keyed by tool name, so
`${tools.fulfil_order}` is both a description and a presence check.

### `browse_mcp_call` — Call a WebMCP tool

```toml
{ action = "browse_mcp_call", tool = "fulfil_order", arguments = { order_id = "${order_id}" }, save_to = "receipt" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `tool` | — | Tool name, as listed by the page. Required |
| `arguments` | `{}` | Inline table passed to the tool as JSON. Golem `${…}` variables resolve inside it |

A tool the page doesn't register fails with `F404`.

The `{ content: [{ type: "text", … }] }` envelope MCP tools return is unwrapped
— a flow gets the answer, not the scaffolding — and a result that is itself JSON
nests, so `${receipt.order_id}` works.

**Availability.** golem enables WebMCP automatically for flows that contain a
`browse_mcp_*` step. It needs an `https://` or `localhost` page. A browser too
old to support it fails with `H505`.

### `browse_set_cookie` / `browse_get_cookie` — Cookies

```toml
{ action = "browse_set_cookie", name = "session", value = "${portal_session}" }
{ action = "browse_set_cookie", name = "region", value = "eu", domain = "portal.example.com", path = "/" }
{ action = "browse_get_cookie", name = "session", save_to = "portal_session" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `name` | — | Cookie name. Required |
| `value` | — | Required for `browse_set_cookie` |
| `domain` | current page | Restrict the cookie to a domain |
| `path` | current page | Restrict the cookie to a path |

`browse_get_cookie` reads `HttpOnly` cookies too. A `browse_get_cookie` for a
name that isn't set fails with `F404`.

### `browse_set_local_storage` / `browse_get_local_storage` — Local storage

```toml
{ action = "browse_set_local_storage", key = "feature_flags", value = "{\"beta\": true}" }
{ action = "browse_get_local_storage", key = "session", save_to = "session" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `key` | — | Required |
| `value` | — | Required for the setter |

**Reading parses JSON objects.** Web apps keep structured state in storage as
JSON text, so an object nests and `${session.user.id}` works. Anything else — an
array, a number, plain text — stays the text it was. A key that isn't set fails
with `F404`.

### `browse_set_session_storage` / `browse_get_session_storage` — Session storage

```toml
{ action = "browse_set_session_storage", key = "step", value = "2" }
{ action = "browse_get_session_storage", key = "step", save_to = "step" }
```

Identical to the local-storage pair, against `sessionStorage`.

### `browse_close` — Close a tab early

```toml
{ action = "browse_close" }
{ action = "browse_close", session = "admin" }
```

Optional. Every tab is closed when the flow ends; this hands one back sooner.

## Flow Control

End the flow on purpose.

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
