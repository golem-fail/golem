### `browse_read` — Read text or an attribute into a variable

```toml
{ action = "browse_read", selector = "#order-total", save_to = "total" }
{ action = "browse_read", selector = "#order-total", attribute = "data-total", save_to = "total_raw" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `attribute` | — | Read this attribute instead of the element's text. Useful when the rendered text is formatted for humans (`£14.99`) and the page already carries the value you want (`data-total="1499"`) |

Fails if the element has no such attribute, rather than saving an empty string.
