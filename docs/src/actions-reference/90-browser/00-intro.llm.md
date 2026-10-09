## Browser

Drive a host browser, for web state that the app depends on.

`browse_*` steps drive a host Chrome/Chromium (or `$CHROME`); none found → plan fails with `H424`. Each flow gets its own browser, closed when the flow ends.

Targeting is **CSS only** (`selector`); mobile selectors (`text`, `on_below`, …) are ignored. Steps check DOM presence, not visibility: an element hidden by CSS counts as present.

| Field | Default | Description |
|-------|---------|-------------|
| `selector` | — | CSS selector. Required by `browse_tap`, `browse_type`, `browse_read`, `browse_select`, `browse_scroll_to`, `browse_assert_*`, `browse_wait_*` |
| `index` | `0` | Which match to act on, 0-based |
| `session` | `_default` | `[context:]tab` |
| `timeout` | `5000`; `10000` for `browse_wait_*` | ms to keep looking for the element |

Tabs in one context share cookies; separate contexts share nothing. `session = "admin"` is a tab in the default context; `session = "tenantB:admin"` is tab `admin` in context `tenantB`. Labels may not contain `:`.

```toml
{ action = "browse_navigate", url = "${portal}", session = "tenantA:main" }
{ action = "browse_navigate", url = "${portal}", session = "tenantB:main" }
```

Storage and cookie steps need a real origin (a page loaded by `browse_navigate`); they fail on `about:blank`.
