### `browse_read` — Read text or an attribute into a variable

```toml
{ action = "browse_read", selector = "#order-total", save_to = "total" }
{ action = "browse_read", selector = "#order-total", attribute = "data-total", save_to = "total_raw" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `attribute` | — | Read this attribute instead of the element's text, e.g. `data-total="1499"` where the text says `£14.99` |
| `save_to` | — | Variable to save the value in |

Fails with `F404` if the element has no such attribute, rather than saving an empty string.
