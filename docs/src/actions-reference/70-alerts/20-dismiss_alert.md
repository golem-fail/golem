### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No, Don't Allow) on the current alert, in the app or from the OS (an iOS permission prompt, for example).

```toml
{ action = "dismiss_alert" }
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a dialog that may not appear.
