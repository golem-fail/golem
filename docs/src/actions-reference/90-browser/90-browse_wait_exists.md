### `browse_wait_exists` — Wait for an element to appear

```toml
{ action = "browse_wait_exists", selector = ".order-row" }
{ action = "browse_wait_exists", selector = "#receipt", timeout = 30000 }
```

Polls until the element is in the DOM. Default `timeout` is 10000ms. Running out
fails with `F408` (step timeout), not `F404`.
