### `hide_keyboard` — Dismiss keyboard

Dismiss the on-screen keyboard. No-op if no keyboard is visible.

```toml
{ action = "hide_keyboard" }
```

You rarely need it to reach a field the keyboard covers: when the keyboard
hides a step's target, golem dismisses the keyboard and looks again. Set
`keep_keyboard = true` on a step to opt out (see
[Step Options](test-structure.md#step-options)):

```toml
{ action = "tap", on_text = "Done", keep_keyboard = true }
```
