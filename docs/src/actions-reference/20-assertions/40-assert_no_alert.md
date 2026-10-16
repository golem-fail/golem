### `assert_no_alert` — Assert no alert is displayed

Verify that no alert or dialog shows, from the app or from the OS. It waits for a showing alert to go, up to the step's `timeout`. With `on_text`, only an alert whose text matches the glob fails it.

```toml
{ action = "assert_no_alert" }
{ action = "assert_no_alert", on_text = "Error*" }
```
