### `browse_scroll_by` — Scroll by a distance

```toml
{ action = "browse_scroll_by" }                                  # 300px down, the page
{ action = "browse_scroll_by", direction = "up", amount = 800 }
{ action = "browse_scroll_by", container = ".results", amount = 500 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `direction` | `"down"` | `up`, `down`, `left` or `right` — same values and default as the mobile `swipe`/`scroll` actions |
| `amount` | `300` | Distance in CSS pixels |
| `container` | — | Scroll this element instead of the window |

Named after the DOM's `scrollBy`, and **not** called `browse_scroll`: the mobile
`scroll` action keeps swiping until an element appears, and that search has no
meaning here — a CSS selector reaches an element whether or not it's on screen.
`_by` and `_to` say which of the two jobs each action does.

`container` rather than `selector`, because every other browser action uses
`selector` for the element the step acts on; here the scrolled element is the
scenery, not the subject.
