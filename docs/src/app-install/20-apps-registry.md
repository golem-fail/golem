## Project-level `[[apps]]` registry
<!-- summary: Declare each app once in golem.toml [[apps]]; flows inherit it -->

Declare each app once in `golem.toml`. Flows reference by `name` and inherit everything else:

```toml
# golem.toml
[[apps]]
name = "app"
bundle = "com.example.app"
install_script = { ios = "scripts/install-app-ios.sh", android = "scripts/install-app-android.sh" }
install_timeout_ms = 900000   # optional override (default 600000 = 10 min)
```

```toml
# flow.test.toml
[[flow.apps]]
name = "app"       # inherits bundle + install_script + install_timeout_ms from [[apps]]

[[flow.apps.devices]]
os = "ios:latest"
type = "phone"
```

Flow-level fields override project-level ones when both are set.
