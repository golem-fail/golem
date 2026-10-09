### Launch-time permissions

An app entry may declare `permissions`. golem grants them **before the app is launched**, so they are in place from process start. A grant that fails is a warning, not a failure: the app then shows its permission prompt when it needs the permission.

```toml
[[flow.apps]]
name = "app"
permissions = { photos = "allow", camera = "deny" }
```

Keys use the cross-platform permission names (`camera`, `microphone`, `location`, `photos`, …). The value is a **mode**: `"allow"` or `"deny"` for any permission, plus `location = "always"` (background; `allow` is foreground) and `photos = "limited"`. To change a permission later in a flow, relaunch the app with the `launch` action's own `permissions =` map. [App permissions](actions-reference.md#app-permissions) owns the full permission and mode reference.
