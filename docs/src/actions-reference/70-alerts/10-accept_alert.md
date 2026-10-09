### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes, Allow) on the current alert. It also handles OS prompts, such as permission requests and "Open in …?" dialogs.

```toml
{ action = "accept_alert" }
{ action = "accept_alert", if_fail = "ignore" }   # prompt may not appear
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a prompt that may not appear.
