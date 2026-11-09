### App permissions
<!-- summary: Grant, deny or reset camera, photos, location, notifications, tracking and other permissions -->

Set permissions with a `permission = mode` map, in `[[flow.apps]].permissions` (applied before the first launch) or on a `launch` step (always cold-starts the app). There is no standalone permission step.

```toml
{ action = "launch", app = "app", permissions = { camera = "allow" } }
{ action = "launch", app = "app", permissions = { camera = "deny", location = "always" } }
```

| Mode | Applies to | Meaning |
|------|-----------|---------|
| `allow` | any permission | granted. `location = "allow"` is foreground only. |
| `deny` | any permission | refused: no prompt, the app sees it denied |
| `ask` | any permission | not asked yet: the app's next request shows the OS prompt |
| `always` | `location` only | background + foreground location |
| `limited` | `photos` only | partial photo-library access |

Any other mode/permission pairing is a parse error.

Permission names: `camera`, `microphone`, `location`, `contacts`, `calendar`, `photos`, `notifications`, `tracking`. Unknown names are an error. Android also accepts a full `android.permission.*` string.

To test a permission prompt, set it to `ask` first, so the prompt shows on every run:

```toml
{ action = "launch", app = "app", permissions = { notifications = "ask" } }
{ action = "tap", on_text = "Enable Notifications" }
{ action = "accept_alert" }
```

- `tracking` (App Tracking Transparency) is iOS only; on Android it sets nothing.
- Android `notifications` is a runtime permission from Android 13; below it `allow`/`ask` set nothing and `deny` is an error.
- iOS permissions need a simulator; on a physical iPhone they are an error (use `hardware = "virtual"` or branch on `_hardware`).

Android: the app's `AndroidManifest.xml` must declare every permission you grant (`photos` needs `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED` and `READ_EXTERNAL_STORAGE`; `location = "always"` also needs `ACCESS_BACKGROUND_LOCATION`; `notifications` needs `POST_NOTIFICATIONS`).

iOS `notifications` and `photos` need `applesimutils`. Without it `notifications` is an error, and `photos` warns and the app prompts at runtime; add this after the step that triggers photo access:

```toml
{ action = "accept_alert", if_fail = "ignore" }
```
