### `push_notification` — Deliver a push to the app under test

Deliver a push notification to the app under test, without APNs or FCM.

```toml
{ action = "push_notification", title = "New message", body = "Hello!" }
```

| Field | Description |
|-------|-------------|
| `title` | Notification title |
| `body` | Notification body |
| `payload` | Optional. iOS only: a string holding a JSON object, added to the APNs payload as `custom`. Ignored on Android. |

Simulator and emulator only, on both platforms. Gate the step on `_hardware` when the flow may run on real hardware:

```toml
[[block.branch]]
if_var = "_hardware"
equals = "virtual"
goto = "send_push"
```

Set `hardware = "virtual"` on the app to silence the real-hardware lint warning.

The app needs its own receive bridge: a `UNUserNotificationCenterDelegate` on iOS; on Android a `BroadcastReceiver` for `<package>.PUSH_NOTIFICATION` reading the `title` and `body` string extras.
