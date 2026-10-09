### `assert_alert` — Assert alert is displayed

Verify an alert/dialog is showing. Optionally match alert text with a glob pattern.

```toml
{ action = "assert_alert" }
{ action = "assert_alert", on_text = "Are you sure*" }
```
