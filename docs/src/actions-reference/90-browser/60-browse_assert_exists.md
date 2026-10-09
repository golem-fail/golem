### `browse_assert_exists` — The element is in the DOM

```toml
{ action = "browse_assert_exists", selector = ".order-row[data-state='fulfilled']" }
```

Waits up to `timeout` for the element to appear. Fails with `F404` if it never does.
