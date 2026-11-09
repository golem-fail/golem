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
| `allow` | any permission | granted. For `location`, foreground only ("when in use"). |
| `deny` | any permission | refused: the app gets no prompt and sees the permission denied |
| `ask` | any permission | not asked yet: the app's next request shows the OS prompt |
| `always` | `location` only | grant background + foreground location |
| `limited` | `photos` only | partial photo-library access (iOS limited library; Android 14+ user-selected subset) |

An invalid mode for a permission (e.g. `camera = "limited"`) is a parse-time error.

**Permissions.** One vocabulary for both platforms: `camera`, `microphone`, `location`, `contacts`, `calendar`, `photos`, `notifications`, `tracking`. An unknown name is an error. On Android you can also pass a full `android.permission.*` string.

**Test a permission prompt** with `ask`. The prompt shows again on every run, whatever the device answered before:

```toml
{ action = "launch", app = "app", permissions = { notifications = "ask" } }
{ action = "tap", on_text = "Enable Notifications" }
{ action = "accept_alert" }
```

**Platform differences.** One flow serves both platforms; golem handles these:

- **`tracking`** is the iOS App Tracking Transparency prompt. Android has none, so on Android `tracking` sets nothing.
- **`notifications` on Android** is a runtime permission from Android 13. Below it notifications are on and the app never asks: `allow` and `ask` set nothing, and `deny` is an error.
- **`deny` and `ask` on `location`** cover background location too.
- **iOS needs a simulator.** On a physical iPhone, setting a permission is an error. Run the flow with `hardware = "virtual"`, or branch on `_hardware`.

Your app's `AndroidManifest.xml` must declare every permission you grant. For `photos`, declare `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED` and `READ_EXTERNAL_STORAGE`: golem grants a different one per Android version. `location = "always"` also needs `ACCESS_BACKGROUND_LOCATION`, and `notifications` needs `POST_NOTIFICATIONS`.

> **iOS `notifications` and `photos` need [`applesimutils`](https://github.com/wix/AppleSimulatorUtils)** (`brew tap wix/brew && brew install wix/brew/applesimutils`); `golem doctor` flags it if missing. Without it, `notifications` is an error. `photos = "allow"` or `"limited"` only warns, and the app prompts at runtime: add `{ action = "accept_alert", if_fail = "ignore" }` after the step that triggers photo access.
