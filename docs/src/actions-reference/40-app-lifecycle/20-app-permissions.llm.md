### App permissions

Set permissions with a `permission = mode` map, in `[[flow.apps]].permissions` (applied before the first launch) or on a `launch` step (always cold-starts the app). There is no standalone permission step.

```toml
{ action = "launch", app = "app", permissions = { camera = "allow" } }
{ action = "launch", app = "app", permissions = { camera = "deny", location = "always" } }
```

| Mode | Applies to | Meaning |
|------|-----------|---------|
| `allow` / `deny` | any permission | grant / explicitly denied. `location = "allow"` is foreground only. |
| `always` | `location` only | background + foreground location |
| `limited` | `photos` only | partial photo-library access |

Any other mode/permission pairing is a parse error.

Permission names: `camera`, `microphone`, `location`, `contacts`, `calendar`, `photos`. Unknown names are an error. Android also accepts a full `android.permission.*` string.

Android: the app's `AndroidManifest.xml` must declare every permission you grant (`photos` needs `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED` and `READ_EXTERNAL_STORAGE`; `location = "always"` also needs `ACCESS_BACKGROUND_LOCATION`).

iOS `photos` grants without a prompt only when `applesimutils` is installed. Without it, the app prompts at runtime; add this after the step that triggers photo access:

```toml
{ action = "accept_alert", if_fail = "ignore" }
```

Notifications can't be pre-granted. Trigger the request in the app, then accept the system dialog:

```toml
{ action = "tap", on_text = "Enable Notifications" }
{ action = "accept_alert", if_fail = "ignore" }
```
