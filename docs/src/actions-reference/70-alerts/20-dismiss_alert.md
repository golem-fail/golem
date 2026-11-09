### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No, Don't Allow) on the current alert, in the app or from the OS (an iOS permission prompt, for example).

```toml
{ action = "dismiss_alert" }
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a dialog that may not appear.

On iOS, an OS prompt is refused by the label of its button, not its position: "Don't Allow", "Ask App Not to Track" (the tracking prompt), "Cancel", "Not Now", "Deny" or "No", the first one the prompt has. If it has none of them, the step fails at once and the error lists the prompt's buttons.
