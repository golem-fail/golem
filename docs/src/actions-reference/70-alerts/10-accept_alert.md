### `accept_alert` — Accept dialog

Tap the button that accepts the current alert, in the app or from the OS (permission requests, "Open in …?").

```toml
{ action = "accept_alert" }
{ action = "accept_alert", if_fail = "ignore" }        # prompt may not appear
{ action = "accept_alert", buttons = ["Autoriser"] }   # labels to try first
```

The button is the first match, in this order:
1. `buttons`, in order: for custom dialogs, rare prompts, or other device languages.
2. Android: the resource id, in any language: a permission prompt's grant (Allow > While using the app > Only this time), or a dialog's positive button.
3. Built-in labels, in order: "Allow Full Access", "Allow Access to All Photos", "Allow While Using App", "While using the app", "Allow", "Allow Once", "Only this time", "Open", "OK", "Yes", "Continue", "Confirm", "Done".
4. The alert's only button.

Labels match whole, ignoring case and `'`/`’`. No match fails at once, listing the buttons and the step with `buttons` to paste. No alert before the timeout fails too; use `if_fail = "ignore"` for a prompt that may not appear.
