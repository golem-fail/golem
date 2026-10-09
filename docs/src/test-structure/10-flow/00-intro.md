## Flow

A flow is a complete test scenario: metadata, app configuration, device targets, execution blocks, and optional teardown.

```toml
[flow]
name = "Login test"
tags = ["auth", "smoke"]
# start = "block_name"  # Optional: skip to this block (assumes app in correct state)

[[flow.apps]]
name = "app"
bundle = "com.example.myapp"

[[flow.apps.devices]]
os = "ios:latest"
type = "phone"

[[flow.apps.devices]]
os = "android:latest"
type = "phone"

[[block]]
name = "login"
steps = [
  { action = "type", on_text = "Email", input = "user@example.com" },
  { action = "type", on_text = "Password", input = "secret" },
  { action = "tap", on_text = "Sign In" },
  { action = "assert_visible", on_text = "Dashboard", timeout = 10000 },
]
```

The flow runs on every device listed. Golem launches the first app automatically before executing blocks.

The `name` you give an app here is the **canonical reference** for it from any action with an `app` field (`launch`, `stop`, `clear_data`, `push_notification`, etc.). Use the name — `app = "app"` — rather than the bundle id. Bundle ids are accepted as a fallback but they're an implementation detail; the name keeps flows readable and survives bundle-id renames. The [Multi-App Flows](#multi-app-flows) section shows the pattern.

#### Launch-time permissions

An app entry may declare `permissions` — granted **before the app is launched**, so they're in place from process start (iOS TCC / Android runtime grants don't apply to an already-running process). Pre-grant is best-effort and a missing one is a warning, not a failure: the app simply prompts when it needs the permission. iOS `photos` needs [`applesimutils`](actions-reference.md#app-permissions) to grant prompt-free (`simctl` can't suppress the iOS 26 library prompt); without it the prompt fires and `accept_alert` covers it.

```toml
[[flow.apps]]
name = "app"
permissions = { photos = "allow", camera = "deny" }
```

Keys use the cross-platform permission vocabulary (`camera`, `microphone`, `location`, `photos`, …); the value is a **mode** — `"allow"`/`"deny"` for any permission, plus `location = "always"` (background; `allow` is foreground) and `photos = "limited"`. See [App permissions](actions-reference.md#app-permissions) for the full mode/permission matrix. To **change** a permission later in a flow, relaunch the app with the `launch` action's own `permissions =` map — a hard restart, the only reliable way to re-apply an iOS grant.
