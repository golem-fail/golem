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
