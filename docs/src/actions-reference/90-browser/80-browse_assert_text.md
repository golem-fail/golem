### `browse_assert_text` — The element says what you expect

```toml
{ action = "browse_assert_text", selector = "#status", text = "Fulfilled" }
{ action = "browse_assert_text", selector = "#total", text = "Total: *" }
{ action = "browse_assert_text", selector = "#total", attribute = "data-total", text = "1499" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | — | Expected value. Supports the same `*` / `?` [glob matching](selectors.md) as mobile text matchers, and is case-sensitive |
| `attribute` | — | Assert on this attribute instead of the element's text |

Polls until the text matches or `timeout` runs out, so a page that updates after
a click isn't judged on what it said beforehand. The failure (`F412`) quotes what
the page actually said.
