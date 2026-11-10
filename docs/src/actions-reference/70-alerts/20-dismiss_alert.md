### `dismiss_alert` — Dismiss dialog

Tap the button that refuses the current alert, in the app or from the OS (an iOS permission prompt, for example).

```toml
{ action = "dismiss_alert" }
{ action = "dismiss_alert", buttons = ["Plus tard"] }   # labels to try first
```

The button is the first match, in this order:
1. `buttons`, in order: for custom dialogs, rare prompts, or other device languages.
2. Android: the resource id, in any language: a permission prompt's "Don't allow", or a dialog's negative button.
3. Built-in labels, in order: "Don't Allow", "Ask App Not to Track", "Cancel", "Not Now", "Deny", "No", "Close", "Dismiss".

Labels match whole, ignoring case and `'`/`’`. No match fails at once, listing the buttons and the step with `buttons` to paste. No alert before the timeout fails too; use `if_fail = "ignore"` for a dialog that may not appear.
