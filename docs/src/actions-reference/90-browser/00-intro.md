## Browser

Drive a host browser, for web state that the app depends on.

Host-side browser automation, for flows whose mobile app depends on web state
nothing else can reach — a supplier fulfilling an order through a portal with no
API, an admin console that flips a feature flag.

**The browser is instrumentation, not the system under test.** The mobile app is
what golem tests, so browser steps are not judged for coverage and never feed
the accessibility audit. They check **DOM presence, not visibility**: an element
hidden by CSS still counts as present, unlike the mobile
[visible tree](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints).

Targeting is **CSS only**. golem's mobile selectors (`text`, `on_below`, and the
rest) describe a native view tree and are ignored by a browser step.

| Field | Default | Description |
|-------|---------|-------------|
| `selector` | — | CSS selector, passed to the page verbatim. Required by the actions that act on an element: `browse_tap`, `browse_type`, `browse_read`, `browse_select`, `browse_scroll_to`, and the `browse_assert_*` and `browse_wait_*` actions |
| `index` | `0` | Which match to act on when the selector matches several, 0-based — same numbering as [`on_index`](selectors.md) |
| `session` | `_default` | `[context:]tab`. Tabs share a context's cookies, so a login carries between them; separate contexts share nothing |
| `timeout` | `5000`; `10000` for `browse_wait_exists` and `browse_wait_not_exists` | How long to keep looking for the element, in ms |

**Requires a Chrome or Chromium on the host**, or `$CHROME` pointing at one;
golem never downloads one. A browser flow on a machine without one fails at plan
time with `H424`, before any device boots. A suite with no `browse_*` step never
looks for one.
Each flow gets its own browser, so concurrent flows never share cookies or
storage, and it is closed when the flow ends whether it passed or failed.

Storage and cookies need a real origin: a page reached by `browse_navigate` has
one, but `about:blank` doesn't, and storage and cookie steps fail there.

**Tabs and contexts.** `session = "admin"` opens a named tab. `session =
"tenantB:admin"` opens that tab in a separate **context** — its own cookie jar —
which is what the same site logged in as two different users at once requires,
since tabs share a login:

```toml
{ action = "browse_navigate", url = "${portal}", session = "tenantA:main" }
{ action = "browse_navigate", url = "${portal}", session = "tenantB:main" }
# tenantA and tenantB can now hold different sessions on the same domain
```

Contexts are created on first use and closed with the flow. `:` is the
separator, so neither label may contain one. There is no sticky context: a step
without the prefix uses the flow's default one.
