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

These go through CDP, not `document.cookie` — which is the point: the cookie a
portal login hands out is usually `HttpOnly`, and script can neither read nor
write those. A `browse_get_cookie` for a name that isn't set fails with `F404`.
