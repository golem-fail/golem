### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No) on the current in-app alert. It may not reach an OS prompt (an iOS permission dialog, for example); `accept_alert` handles those.

```toml
{ action = "dismiss_alert" }
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a dialog that may not appear.
