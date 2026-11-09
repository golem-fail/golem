### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No, Don't Allow) on the current alert, in the app or from the OS (an iOS permission prompt, for example).

```toml
{ action = "dismiss_alert" }
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a dialog that may not appear.

An OS prompt is answered by its button, never by position:
- iOS: the first of "Don't Allow", "Ask App Not to Track", "Cancel", "Not Now", "Deny", "No".
- Android: by resource id, so any device language: a permission prompt's "Don't allow", or a dialog's negative button. A dialog without ids: its first refusing label ("Cancel", "No", …), else its first button.

A prompt with none of these fails at once and lists its buttons.
