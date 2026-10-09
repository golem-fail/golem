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
