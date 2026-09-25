# Actions Reference

*Every word the golem knows.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) for selectors, steps, and flow anatomy.

## Contents

- [Interaction](#interaction)
  - [tap](#tap--tap-an-element)
  - [double_tap](#double_tap--double-tap-an-element)
  - [type](#type--type-text-into-an-element)
  - [backspace](#backspace--delete-characters)
  - [clear_text](#clear_text--empty-a-field)
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
  - [App permissions](#app-permissions)
  - [stop](#stop--terminate-an-app)
  - [clear_data](#clear_data--clear-app-data)
- [Device Controls](#device-controls)
  - [set_dark_mode](#set_dark_mode--set-dark-mode)
  - [set_location](#set_location--set-gps-coordinates)
  - [press](#press--press-hardware-button)
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
  - [load_mixin](#load_mixin--inline-a-reusable-step-sequence)
  - [*_http](#get_http-post_http-put_http-patch_http-delete_http--http-requests)
- [Browser](#browser)
  - [browse_navigate](#browse_navigate--load-a-url)
  - [browse_tap](#browse_tap--click-an-element)
  - [browse_type](#browse_type--type-into-a-field)
  - [browse_read](#browse_read--read-text-or-an-attribute-into-a-variable)
  - [browse_screenshot](#browse_screenshot--capture-the-tab)
  - [browse_assert_exists](#browse_assert_exists--the-element-is-in-the-dom)
  - [browse_assert_not_exists](#browse_assert_not_exists--the-element-is-not-in-the-dom)
  - [browse_assert_text](#browse_assert_text--the-element-says-what-you-expect)
  - [browse_wait_exists](#browse_wait_exists--wait-for-an-element-to-appear)
  - [browse_wait_not_exists](#browse_wait_not_exists--wait-for-an-element-to-disappear)
  - [browse_scroll_by](#browse_scroll_by--scroll-by-a-distance)
  - [browse_scroll_to](#browse_scroll_to--bring-an-element-into-view)
  - [browse_select](#browse_select--choose-an-option-in-a-select)
  - [browse_execute_js](#browse_execute_js--run-javascript-in-the-page)
  - [browse_mcp_list_tools](#browse_mcp_list_tools--list-the-pages-webmcp-tools)
  - [browse_mcp_call](#browse_mcp_call--call-a-webmcp-tool)
  - [browse_set_cookie / browse_get_cookie](#browse_set_cookie--browse_get_cookie--cookies)
  - [browse_set_local_storage / browse_get_local_storage](#browse_set_local_storage--browse_get_local_storage--local-storage)
  - [browse_set_session_storage / browse_get_session_storage](#browse_set_session_storage--browse_get_session_storage--session-storage)
  - [browse_close](#browse_close--close-a-tab-early)
- [Flow Control](#flow-control)
  - [fail](#fail--fail-the-flow-immediately)

<!-- The canonical list of action keywords is the dispatch match in [`golem-runner/src/actions.rs`](../golem-runner/src/actions.rs), plus [`golem-browser/src/actions.rs`](../golem-browser/src/actions.rs) for the `browse_*` family. If you add a handler in either, document it here AND list it in the Contents above — `actions_reference_doc_lists_every_action` and `actions_reference_contents_links_every_entry` enforce both. -->

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

> **iOS timing note.** A `tap` is synthesised as `press(forDuration: 0.05)`
> (50 ms), not a bare `tap()`. The bare call emits touch-up immediately after
> touch-down, which a WebView can race-drop — leaving the click unfired. The
> 50 ms hold makes XCUITest serialise down → hold → up reliably. The trade-off:
> a page whose long-press recogniser triggers below ~50 ms may classify a
> `tap` as a long-press. In that rare case use an explicit `long_press` (or a
> coordinate tap) to disambiguate.

### `double_tap` — Double-tap an element

Two rapid taps (40ms apart) at the element center.

```toml
{ action = "double_tap", on_text = "Zoom" }
```

Same selectors and options as `tap`.

### `type` — Type text into an element

With a selector, taps the element to focus it, then types the `input`
string. The selector is **optional**: with no selector, `type` sends the
keystrokes to the currently focused field without tapping — useful for
appending to the field the previous step left focused (the caret stays at
the end), or for apps that respond to keypresses outside a text input.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
{ action = "type", input = " and more" }   # append to the focused field
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |

### `backspace` — Delete characters

Deletes `count` characters from the **currently focused** text field. It
takes **no selector** — `type` or `tap` the field
first; the caret is left at the end of the text, so backspace removes from
there. A selector is rejected: a tap-to-focus would re-place the caret at the
tap point (mid-text on a filled field, deleting the wrong char), and there is
no reliable cross-platform way to move the caret to the end.

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

golem reads the focused field's length from the hierarchy and deletes exactly
that many characters, so no companion-side "select all" is involved. Two
consequences worth knowing:

- **The caret must be at the end.** Deletes only remove what is behind the
  caret, so a caret left mid-field can't reach the tail. golem detects this and
  fails the step telling you to re-focus, rather than silently half-clearing.
  `type` leaves the caret at the end; a `tap` places it where you tapped.
- **A field whose contents exactly equal its placeholder reads as empty** and is
  left alone. An empty field reports its placeholder as the text the user sees,
  and the two cases are indistinguishable on the wire.

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

> **Automatic keyboard recovery.** You rarely need an explicit
> `hide_keyboard` to reach a field the keyboard covers. During element
> resolution, if a target is absent from the keyboard-aware viewport but
> present in the unfiltered tree (i.e. the soft keyboard has occluded it),
> the resolver dismisses the keyboard **once per resolve** and re-polls. On
> Android the IME hide is animated and asynchronous, so the resolver waits
> for the reported keyboard height to return to 0 (with a timeout) before
> retrying, so the retap lands on the now-revealed field rather than the
> still-sliding panel.
>
> Set `keep_keyboard = true` on a step to opt out — of this recovery and of
> the pre-tap dismissal both. Use it when the step targets a keyboard
> accessory/toolbar control that acts on the focused field, or when the test
> is *about* keyboard-up state. An occluded target then stays unresolved
> rather than being reached by dismissing the keyboard:
>
> ```toml
> { action = "tap", on_text = "Done", keep_keyboard = true }
> ```

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
{ action = "launch", app = "app", restart = true }                        # Kill and relaunch
{ action = "launch", app = "app", permissions = { camera = "allow" } }    # Grant, then cold start
```

| Field | Default | Description |
|-------|---------|-------------|
| `app` | — | App name (as defined in `[[flow.apps]]`) |
| `restart` | `false` | Stop app first, then launch fresh |
| `permissions` | — | Per-launch permission map (see [App permissions](#app-permissions)) |

A `permissions` map on `launch` **always cold-starts** the app, even without `restart = true`: iOS TCC only applies a grant to a *stopped* process, so golem stops the app, applies the grant/revoke, then launches fresh. A soft foreground would read stale permission state. Use this to flip a permission mid-flow — relaunching with a new map is the supported way to test both the granted and denied paths in one flow.

### App permissions

Permissions are declared **on a launch**, not as a standalone step — the platform tooling (`pm grant` on Android, `simctl privacy` / `applesimutils` on iOS sims) terminates the app to apply a grant, so a permission change is inseparable from a (re)launch. Two places take the same `permission = mode` map:

- **`[[flow.apps]].permissions`** — the baseline, applied once before the app's first launch (see [Test structure](test-structure.md#launch-time-permissions)).
- **`launch` action `permissions =`** — a per-launch override for changing a permission later in the flow (a cold start, as above).

```toml
{ action = "launch", app = "app", permissions = { camera = "allow" } }
# ... exercise the granted path ...
{ action = "launch", app = "app", permissions = { camera = "deny" } }
# ... exercise the denied path ...
```

> **Migrating from `grant_permission` / `revoke_permission`:** those step-level actions were removed. Replace `{ action = "grant_permission", app = "app", permission = "camera" }` immediately followed by a `launch` with a single `{ action = "launch", app = "app", permissions = { camera = "allow" } }` (`"deny"` for revoke). They always forced an app restart anyway, so this is a faithful — and shorter — replacement.

**Modes.** The value is a mode, not just on/off:

| Mode | Applies to | Meaning |
|------|-----------|---------|
| `allow` / `deny` | any permission | grant / **explicit-denied** (`simctl privacy revoke` / `pm revoke`, not a not-determined reset). For `location`, `allow` grants foreground ("when in use"). |
| `always` | `location` only | grant background + foreground location (the broader grant; `allow` is foreground-only) |
| `limited` | `photos` only | partial photo-library access (iOS limited library; Android 14+ user-selected subset) |

An invalid mode for a permission (e.g. `camera = "limited"`) is a parse-time error.

**Cross-platform permissions.** One vocabulary, mapped per platform:

| Permission | Android (`pm grant`)                                      | iOS (`simctl privacy`) |
|------------|-----------------------------------------------------------|------------------------|
| `camera`   | `CAMERA`                                                  | `camera`               |
| `microphone` | `RECORD_AUDIO`                                          | `microphone`           |
| `location` | `ACCESS_FINE_LOCATION` (+ `ACCESS_BACKGROUND_LOCATION` when `= "always"`) | `location` / `location-always` (per mode) |
| `contacts` | `READ_CONTACTS`                                           | `contacts`             |
| `calendar` | `READ_CALENDAR`                                           | `calendar`             |
| `photos`   | SDK-conditional: `READ_MEDIA_IMAGES` (+ `…_VISUAL_USER_SELECTED` on Android 14+; `= "limited"` grants only the latter) / `READ_EXTERNAL_STORAGE` on Android 12 and below | `photos` (via `applesimutils` — see below) |

Unknown permissions fail loudly (no silent passthrough). You can also pass a full `android.permission.*` string and Android will use it verbatim.

> **iOS photos needs `applesimutils`.** `simctl privacy grant photos` accepts the command but does **not** suppress the iOS 26 full-library-access prompt, so golem routes `photos` pre-grants through [`applesimutils`](https://github.com/wix/AppleSimulatorUtils) (`brew tap wix/brew && brew install wix/brew/applesimutils`), which does. It's optional-but-recommended — `golem doctor` flags it. Without it, a `photos` grant can't be applied prompt-free: golem warns and the app prompts at runtime, so add `{ action = "accept_alert", if_fail = "ignore" }` after the step that triggers photo access. All other iOS permissions use `simctl privacy` and need no extra tooling.

> **Note: notifications aren't a pre-grantable shorthand.** Both iOS and Android (13+) show a system dialog the first time the app calls the notification-authorization API — pre-granting is Android-only and breaks parity. The cross-platform pattern is to trigger the request from inside the app and dismiss the dialog with `accept_alert`:
>
> ```toml
> { action = "tap", on_text = "Enable Notifications" }
> { action = "accept_alert", if_fail = "ignore" }
> ```
>
> `if_fail = "ignore"` keeps the step happy on warm sims/emulators that have already recorded the user's prior choice and skipped the prompt.

Your app's `AndroidManifest.xml` must declare every permission you intend to grant — `pm grant` rejects undeclared permissions, and all three photo permissions need declaring because the shorthand resolves to a different one per SDK level. golem's own test app is generated by Tauri, which has no config for `<uses-permission>`, so it re-applies the declarations after generation in `scripts/patch-test-app-projects.sh` — that script's list is the set the shorthands above expand to.

### `stop` — Terminate an app

```toml
{ action = "stop", app = "app" }
```

### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

**Simulator-only on iOS; Android works everywhere.** The iOS path clears the app's data container through a host filesystem path that `simctl` hands back, which only exists for a simulator — on a physical device the container lives on the device and `get_app_container` returns a path the host can't reach. Android uses `adb shell pm clear`, which is device-agnostic. On a physical iPhone the driver bails pointing at this paragraph.

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

### `set_dark_mode` — Set dark mode

```toml
{ action = "set_dark_mode", enabled = true }
{ action = "set_dark_mode", enabled = false }
```

### `set_location` — Set GPS coordinates

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

> **The iOS device controls are simulator-backed.** `set_dark_mode`, `set_location` and `add_media` all drive `simctl`, which only addresses simulators — on a physical iPhone the driver refuses, naming the action and the device, rather than surfacing a raw `simctl` error. Gate the step on `_hardware` if a flow has to run on both shapes. The Android equivalents go through `adb` and work on emulators and physical devices alike.

### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

**Supported buttons (platform-specific):**

| `button`      | Android (`input keyevent`) | iOS (`/press` → `XCUIDevice.press`) |
|---------------|----------------------------|-------------------------------------|
| `home`        | ✓ `HOME`                   | ✓ `.home`                           |
| `back`        | ✓ `BACK`                   | — (no hardware back button)         |
| `volume_up`   | ✓ `VOLUME_UP`              | —                                   |
| `volume_down` | ✓ `VOLUME_DOWN`            | —                                   |

An unsupported button errors at action time. On iOS only `home` exists;
`simctl ui … home` was dropped in Xcode 26, so golem drives it through the
companion's `/press` endpoint (`XCUIDevice.shared.press(.home)`), the
version-stable path.

App permissions are declared on a launch, not as a device control — see [App permissions](#app-permissions) under App Lifecycle.

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

**Simulator-only on iOS** — `simctl io recordVideo` has no physical-device equivalent, so a recording request on a real iPhone fails. It degrades rather than breaking the run: the block records a warning and its steps execute normally, just without a video. Android records on physical devices and emulators alike. Tracked in [#60](https://github.com/golem-fail/golem/issues/60).

### `add_media` — Push media to device

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

**Simulator-only on iOS** — `simctl addmedia` can't address a physical device, so the driver refuses there; put the fixture in the device's library ahead of the run, or gate the step on `_hardware`. Android uses `adb push` plus a media-scanner broadcast and works on physical devices and emulators alike. See [Device Controls](#device-controls) for the other two simulator-backed actions.

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

A `[lint]` warning fires at parse time when a flow uses `push_notification` and any of its apps could be scheduled onto real hardware — `hardware = "real"`, `["virtual", "real"]`, or `hardware` left unspecified (which accepts either shape). It's an early breadcrumb that the action will fail on the phys branch unless you wrap it in `branch` like above. Pin `hardware = "virtual"` to state that the flow is sim/emu-only and silence it.

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
| `recipient` | — | Glob filter for the recipient address. Spelled `recipient`, not `to`: a step-level `to` is the grouped selector alias |
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
text = "${wait_element}"
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

Fails on non-2xx status codes.

## Browser

Host-side browser automation, for flows whose mobile app depends on web state
nothing else can reach — a supplier fulfilling an order through a portal with no
API, an admin console that flips a feature flag.

**The browser is instrumentation, not the system under test.** The mobile app is
what golem tests, so browser steps are not judged for coverage, never feed the
accessibility audit, and assert on DOM presence rather than the
[visible tree](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)
— what a headless browser "sees" is not what a user sees, and pretending
otherwise would be theatre.

Targeting is **CSS only**. golem's mobile selectors (`text`, `on_below`, and the
rest) describe a native view tree and are ignored by a browser step.

| Field | Default | Description |
|-------|---------|-------------|
| `selector` | — | CSS selector, passed to the page verbatim. Required by every action except `browse_navigate` and `browse_screenshot` |
| `index` | `0` | Which match to act on when the selector matches several, 0-based — same numbering as [`on_index`](selectors.md) |
| `session` | `_default` | `[context:]tab`. Tabs share a context's cookies, so a login carries between them; separate contexts share nothing |
| `timeout` | `5000` | How long to keep looking for the element, in ms |

**Requires a Chrome or Chromium on the host.** golem drives whichever one it
finds (`$CHROME` points it at a specific binary) and never downloads one. macOS:
install Google Chrome normally. Debian/Ubuntu: `apt install chromium` or
Google's `google-chrome-stable` package. A suite whose flows contain no
`browse_*` step never looks for one, so a mobile-only run needs nothing
installed — and a browser flow on a machine without one fails at plan time with
`H424`, before any device boots.
Each flow gets its own browser, so concurrent flows never share cookies or
storage, and it is closed when the flow ends whether it passed or failed.

**Tabs and contexts.** `session = "admin"` opens a named tab. `session =
"tenantB:admin"` opens that tab in a separate **context** — its own cookie jar —
which is what the same site logged in as two different users at once requires,
since tabs deliberately share a login:

```toml
{ action = "browse_navigate", url = "${portal}", session = "tenantA:main" }
{ action = "browse_navigate", url = "${portal}", session = "tenantB:main" }
# tenantA and tenantB can now hold different sessions on the same domain
```

Contexts are created on first use and closed with the flow. Labels are
flow-local, so two flows using the same name are already separate browsers. `:`
is the separator, so neither label may contain one. There is no sticky context:
a step without the prefix uses the flow's default one.

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
| `attribute` | — | Read this attribute instead of the element's text. Useful when the rendered text is formatted for humans (`£14.99`) and the page already carries the value you want (`data-total="1499"`) |

Fails if the element has no such attribute, rather than saving an empty string.

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
for something to disappear — that's `browse_wait_not`; retrying here would spend
the whole timeout confirming every absence, which is the case that usually passes.

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

Polls until the element is in the DOM. Default `timeout` is 10000ms here — a
wait is an explicit "this may take a while", unlike the incidental lookup an
ordinary action does.

Running out reports `F408` (step timeout), not `F404`: a wait that expires means
the page never got where the flow expected, while a failed
`browse_assert_exists` means the page is wrong. Both poll identically; they
differ in what the report tells you afterwards.

### `browse_wait_not_exists` — Wait for an element to disappear

```toml
{ action = "browse_wait_not_exists", selector = ".spinner" }
```

The one thing no assertion does: `browse_assert_not_exists` answers "is it gone
now", this answers "let it finish going". Spinners, toasts and progress rows are
the reason it exists.

These wait on **DOM presence**, not visibility — an element hidden by CSS still
counts as present. Browser steps are instrumentation, and visibility judgements
belong to the mobile app under test.

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

Named after the DOM's `scrollBy`, and **not** called `browse_scroll`: the mobile
`scroll` action keeps swiping until an element appears, and that search has no
meaning here — a CSS selector reaches an element whether or not it's on screen.
`_by` and `_to` say which of the two jobs each action does.

`container` rather than `selector`, because every other browser action uses
`selector` for the element the step acts on; here the scrolled element is the
scenery, not the subject.

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

Golem variables are interpolated into `script` but **not** into `file`: a shared
helper shouldn't change meaning depending on which flow imported it. Both are
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

Driving a page's declared tools beats clicking through its UI where they exist:
the page states its own contract, so the flow isn't coupled to a layout that may
be redesigned next quarter. A tool the page doesn't register fails with `F404`.

The `{ content: [{ type: "text", … }] }` envelope MCP tools return is unwrapped
— a flow gets the answer, not the scaffolding — and a result that is itself JSON
nests, so `${receipt.order_id}` works.

**Availability.** WebMCP ships switched off. golem turns it on automatically for
flows containing a `browse_mcp_*` step (it launches the browser, so nothing
needs toggling in `chrome://flags`), and leaves it off otherwise, since an
experimental browser feature changes what every page can feature-detect. It also
needs a **secure origin**: an `https://` or `localhost` page. A browser too old
to support it fails with `H505`.

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

These go through CDP, not `document.cookie` — which is the point: the cookie a
portal login hands out is usually `HttpOnly`, and script can neither read nor
write those. A `browse_get_cookie` for a name that isn't set fails with `F404`.

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

Storage and cookies need a real origin: a page reached by `browse_navigate` has
one, but `about:blank` doesn't, and both will fail there.

### `browse_close` — Close a tab early

```toml
{ action = "browse_close" }
{ action = "browse_close", session = "admin" }
```

Optional. Every tab is closed when the flow ends; this hands one back sooner.

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
