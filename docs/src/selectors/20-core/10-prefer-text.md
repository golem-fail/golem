### Prefer visible `text`; use `accessibility_label` sparingly

**What counts as text.** `text` is the text the platform reports as visible.
When an element has no text of its own, its accessibility label is its text,
on both platforms. Thus an icon-only button labelled `Close` matches
`on_text = "Close"`. An image (Android `ImageView`, iOS `image`) is the
exception: its label describes a picture, so it is never text. In a webview,
`text` is the DOM text only.

golem's premise is **testing like a human** — a human reads and taps *visible
text*, not an accessibility identifier they can't see. So default to `on_text`
(or a positional/`contains` selector). Reach for `on_accessibility_label` only
when:

1. **You are explicitly testing the accessibility label itself** — e.g.
   validating screen-reader semantics / a11y compliance. Here the label *is* the
   thing under test.
2. **As a throwaway shortcut** to *navigate* to the part you actually want to
   test, when the element you're tapping isn't itself the subject of the
   assertion (e.g. opening a menu by its stable `menu-toggle` id so you can get
   to the screen you care about). You're not testing the label, just using it to
   get somewhere.

Outside those cases, an `accessibility_label` selector tests something the user
never perceives, and silently passes even if the visible text is wrong. When in
doubt, use `text`.
