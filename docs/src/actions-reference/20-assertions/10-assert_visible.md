### `assert_visible` — Wait for / assert element exists

Poll the hierarchy until an element matching the selectors is on screen, or `timeout` elapses (default 10s). Use a short `timeout` for instantaneous checks, a long one for waits. The assertion is driven by the selectors — add `on_enabled` / `on_checked` to assert state, not just presence.

```toml
{ action = "assert_visible", on_text = "Welcome" }
{ action = "assert_visible", on_text = "1", on_below = "Counter" }
{ action = "assert_visible", on_text = "Submit", on_enabled = true }                    # state, not just presence
{ action = "assert_visible", on_text = "Item 0", auto_scroll = true, timeout = 60000 }  # off-screen element
```
