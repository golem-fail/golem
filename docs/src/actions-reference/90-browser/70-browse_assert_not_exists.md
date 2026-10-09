### `browse_assert_not_exists` — The element is not in the DOM

```toml
{ action = "browse_assert_not_exists", selector = "#error-banner" }
```

Checks once and fails with `F409` if the element is there. It does **not** wait
for something to disappear — that's `browse_wait_not`; retrying here would spend
the whole timeout confirming every absence, which is the case that usually passes.
