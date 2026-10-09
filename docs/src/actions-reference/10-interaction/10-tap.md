### `tap` — Tap an element

Find an element matching the selectors and tap its center.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "tap", on_text = "+", timeout = 5000 }
{ action = "tap", on = { text = "OK", below = "Confirm?" } }
{ action = "tap", on_accessibility_label = "Increment" }
```

Supports all selectors, `auto_scroll`, `timeout`, `if_fail`, `retry`.

> **iOS timing note.** A `tap` is synthesised as `press(forDuration: 0.05)`
> (50 ms), not a bare `tap()`. The bare call emits touch-up immediately after
> touch-down, which a WebView can race-drop — leaving the click unfired. The
> 50 ms hold makes XCUITest serialise down → hold → up reliably. The trade-off:
> a page whose long-press recogniser triggers below ~50 ms may classify a
> `tap` as a long-press. In that rare case use an explicit `long_press` (or a
> coordinate tap) to disambiguate.
