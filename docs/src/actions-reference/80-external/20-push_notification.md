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
