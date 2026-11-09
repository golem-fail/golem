### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes, Allow) on the current alert. It also handles OS prompts, such as permission requests and "Open in …?" dialogs.

```toml
{ action = "accept_alert" }
{ action = "accept_alert", if_fail = "ignore" }   # prompt may not appear
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a prompt that may not appear.

An OS prompt is answered by its button, never by position:
- iOS: the first of "Allow Full Access", "Allow Access to All Photos", "Open", "Allow", "Allow Once", "Allow While Using App".
- Android: by resource id, so any device language: a permission prompt's grant (Allow > While using the app > Only this time), or a dialog's positive button. A dialog without ids: its last button.

A prompt with none of these fails at once and lists its buttons.
