### Launch-time permissions
<!-- summary: Grant app permissions before the app launches -->

An app entry may declare `permissions`. golem grants them **before the app is launched**, so they are in place from process start. A permission golem can't set fails the flow, with one exception: iOS `photos` without `applesimutils` is a warning, and the app shows its permission prompt when it needs photos.

```toml
[[flow.apps]]
name = "app"
permissions = { photos = "allow", camera = "deny" }
```

Keys use the cross-platform permission names (`camera`, `microphone`, `location`, `photos`, …). The value is a **mode**: `"allow"`, `"deny"` or `"ask"` (not asked yet, so the app prompts) for any permission, plus `location = "always"` (background; `allow` is foreground) and `photos = "limited"`. To change a permission later in a flow, relaunch the app with the `launch` action's own `permissions =` map. [App permissions](actions-reference.md#app-permissions) owns the full permission and mode reference.
