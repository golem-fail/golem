### `browse_wait_not_exists` — Wait for an element to disappear

```toml
{ action = "browse_wait_not_exists", selector = ".spinner" }
```

Polls until the element is gone from the DOM. Default `timeout` is 10000ms.
Running out fails with `F408`. To check absence once without waiting, use
`browse_assert_not_exists`.
