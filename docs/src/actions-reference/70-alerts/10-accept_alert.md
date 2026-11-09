### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes, Allow) on the current alert. It also handles OS prompts, such as permission requests and "Open in …?" dialogs.

```toml
{ action = "accept_alert" }
{ action = "accept_alert", if_fail = "ignore" }   # prompt may not appear
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a prompt that may not appear.

On iOS, an OS prompt is accepted by the label of its button, not its position: "Allow Full Access", "Allow Access to All Photos", "Open", "Allow", "Allow Once" or "Allow While Using App", the first one the prompt has. If it has none of them, the step fails at once and the error lists the prompt's buttons.

On Android, the answer comes from the button's resource id, which does not change with the device language: a permission prompt's grant ("Allow", then "While using the app", then "Only this time"), or a standard dialog's positive button. A permission prompt without one of those fails at once and lists its buttons. A dialog without ids is accepted with its last button.
