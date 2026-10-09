### `browse_navigate` — Load a URL

```toml
{ action = "browse_navigate", url = "https://portal.example.com/orders" }
{ action = "browse_navigate", url = "${portal_url}", wait_until = "load" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `url` | — | Required |
| `wait_until` | `"domcontentloaded"` | When the step returns: `none` (as soon as Chrome accepts it), `domcontentloaded` (the DOM is parsed and queryable), `load` (sub-resources finished too) |
| `user_agent` | the browser's own | Present this session as a different client. Applied **before** the navigation, so the first request carries it |
| `accept_language` | the browser's own | `Accept-Language` for this session, e.g. `"fr-FR"` |

**Identity belongs to the tab, not the step.** `user_agent` and
`accept_language` stay in force for that `session` until something changes
them, so set them on the session's first navigation and later steps inherit
them — a session that is a phone stays a phone. Two sessions can hold different
identities at once:

```toml
{ action = "browse_navigate", url = "${portal}", session = "phone",
  user_agent = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) …" }
{ action = "browse_navigate", url = "${portal}", session = "desktop" }
```

Two things this does **not** do. It doesn't change rendering — layout follows
the viewport, so a page won't reflow to phone width because its user agent says
iPhone. And it doesn't touch [Client Hints](https://developer.chrome.com/docs/privacy-security/user-agent-client-hints):
`Sec-CH-UA-Mobile` and friends still describe the real Chrome, so a portal that
reads those rather than the user-agent string will see through it.
