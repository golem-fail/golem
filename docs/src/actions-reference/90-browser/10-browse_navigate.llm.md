### `browse_navigate` — Load a URL

```toml
{ action = "browse_navigate", url = "https://portal.example.com/orders" }
{ action = "browse_navigate", url = "${portal}", session = "phone", user_agent = "Mozilla/5.0 (iPhone; …)" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `url` | — | Required |
| `wait_until` | `"domcontentloaded"` | `none`, `domcontentloaded`, or `load` (sub-resources finished too) |
| `user_agent` | the browser's own | User agent for this session, applied before the request |
| `accept_language` | the browser's own | `Accept-Language` for this session, e.g. `"fr-FR"` |

`user_agent` and `accept_language` persist for the session: later steps in that `session` keep them. They do not change layout (the viewport does) or Client Hints (`Sec-CH-UA-*`).
