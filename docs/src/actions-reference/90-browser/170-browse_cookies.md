### `browse_set_cookie` / `browse_get_cookie` — Cookies

```toml
{ action = "browse_set_cookie", name = "session", value = "${portal_session}" }
{ action = "browse_set_cookie", name = "region", value = "eu", domain = "portal.example.com", path = "/" }
{ action = "browse_get_cookie", name = "session", save_to = "portal_session" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `name` | — | Cookie name. Required |
| `value` | — | Required for `browse_set_cookie` |
| `domain` | current page | Restrict the cookie to a domain |
| `path` | current page | Restrict the cookie to a path |

`browse_get_cookie` reads `HttpOnly` cookies too. A `browse_get_cookie` for a
name that isn't set fails with `F404`.
