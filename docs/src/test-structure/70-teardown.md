## Teardown

Teardown blocks run after the flow completes, regardless of pass/fail — running **even when the flow fails** is the point: it cleans up external state (test data, created users) that a failed run would otherwise leak. Failures in teardown don't affect the test result (they surface as `Teardown:` warnings on the report). Teardown runs before the automatic device-state reset (dark mode, mocked location, recording), so it still sees the app as the flow left it.

```toml
[[teardown]]
steps = [
  { action = "screenshot", path = "/tmp/final.png" },
  { action = "stop", app = "app" },
]
```

Skip teardown with `--no-teardown`.
