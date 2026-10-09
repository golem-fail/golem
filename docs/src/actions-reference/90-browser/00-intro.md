## Browser

Host-side browser automation, for flows whose mobile app depends on web state
nothing else can reach — a supplier fulfilling an order through a portal with no
API, an admin console that flips a feature flag.

**The browser is instrumentation, not the system under test.** The mobile app is
what golem tests, so browser steps are not judged for coverage, never feed the
accessibility audit, and assert on DOM presence rather than the
[visible tree](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)
— what a headless browser "sees" is not what a user sees, and pretending
otherwise would be theatre.

Targeting is **CSS only**. golem's mobile selectors (`text`, `on_below`, and the
rest) describe a native view tree and are ignored by a browser step.

| Field | Default | Description |
|-------|---------|-------------|
| `selector` | — | CSS selector, passed to the page verbatim. Required by every action except `browse_navigate` and `browse_screenshot` |
| `index` | `0` | Which match to act on when the selector matches several, 0-based — same numbering as [`on_index`](selectors.md) |
| `session` | `_default` | `[context:]tab`. Tabs share a context's cookies, so a login carries between them; separate contexts share nothing |
| `timeout` | `5000` | How long to keep looking for the element, in ms |

**Requires a Chrome or Chromium on the host.** golem drives whichever one it
finds (`$CHROME` points it at a specific binary) and never downloads one. macOS:
install Google Chrome normally. Debian/Ubuntu: `apt install chromium` or
Google's `google-chrome-stable` package. A suite whose flows contain no
`browse_*` step never looks for one, so a mobile-only run needs nothing
installed — and a browser flow on a machine without one fails at plan time with
`H424`, before any device boots.
Each flow gets its own browser, so concurrent flows never share cookies or
storage, and it is closed when the flow ends whether it passed or failed.

**Tabs and contexts.** `session = "admin"` opens a named tab. `session =
"tenantB:admin"` opens that tab in a separate **context** — its own cookie jar —
which is what the same site logged in as two different users at once requires,
since tabs deliberately share a login:

```toml
{ action = "browse_navigate", url = "${portal}", session = "tenantA:main" }
{ action = "browse_navigate", url = "${portal}", session = "tenantB:main" }
# tenantA and tenantB can now hold different sessions on the same domain
```

Contexts are created on first use and closed with the flow. Labels are
flow-local, so two flows using the same name are already separate browsers. `:`
is the separator, so neither label may contain one. There is no sticky context:
a step without the prefix uses the flow's default one.
