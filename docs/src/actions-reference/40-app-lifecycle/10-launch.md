### `launch` — Launch or foreground an app

Bring an app to the foreground. Does not restart if already running. Use `restart = true` for a cold start.

```toml
{ action = "launch", app = "app" }
{ action = "launch", app = "companion" }
{ action = "launch", app = "app", restart = true }                        # Kill and relaunch
{ action = "launch", app = "app", permissions = { camera = "allow" } }    # Grant, then cold start
```

| Field | Default | Description |
|-------|---------|-------------|
| `app` | — | App name (as defined in `[[flow.apps]]`) |
| `restart` | `false` | Stop app first, then launch fresh |
| `permissions` | — | Per-launch permission map (see [App permissions](#app-permissions)) |

A `permissions` map always cold-starts the app, even without `restart = true`. Use it to flip a permission mid-flow and test both the granted and denied paths in one flow.
