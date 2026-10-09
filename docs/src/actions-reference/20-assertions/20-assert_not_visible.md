### `assert_not_visible` — Wait for / assert element absent

Poll the hierarchy until no element matches the selectors, or `timeout` elapses (default 10s).

```toml
{ action = "assert_not_visible", on_text = "Error" }
{ action = "assert_not_visible", on_text = "Loading", timeout = 10000 }
```
