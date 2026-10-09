### `browse_wait_exists` — Wait for an element to appear

```toml
{ action = "browse_wait_exists", selector = ".order-row" }
{ action = "browse_wait_exists", selector = "#receipt", timeout = 30000 }
```

Polls until the element is in the DOM. Default `timeout` is 10000ms here — a
wait is an explicit "this may take a while", unlike the incidental lookup an
ordinary action does.

Running out reports `F408` (step timeout), not `F404`: a wait that expires means
the page never got where the flow expected, while a failed
`browse_assert_exists` means the page is wrong. Both poll identically; they
differ in what the report tells you afterwards.
