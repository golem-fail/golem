### `assert_not_visible` — Wait for / assert element absent

Poll until no element matching the selectors is visible on screen, or `timeout` elapses (default 10s). An element still in the hierarchy but scrolled off screen, clipped by its container, or behind the keyboard counts as not visible.

```toml
{ action = "assert_not_visible", on_text = "Error" }
{ action = "assert_not_visible", on_text = "Loading", timeout = 10000 }
```
