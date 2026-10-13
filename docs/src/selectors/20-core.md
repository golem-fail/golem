## Core selectors
<!-- summary: on_text, on_accessibility_label, on_index; what counts as text; when to prefer text -->

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_text` | `text` | Visible text. Glob (`*`, `?`), case-insensitive, anchored (full-string — use globs for partial: `"Item *"`, `"*@*"`). |
| `on_accessibility_label` | `accessibility_label` | The element's accessibility label (aria-label, Android `contentDescription`) or its identifier (iOS `accessibilityIdentifier`, Android `resource-id`). `golem tree` shows these as `label=` and `id=`. **See the guidance below — prefer `text`.** |
| `on_index` | `index` | The Nth match (0-based) after all other filters. |

**What counts as text.** `text` is the text the platform reports as visible.
When an element has no text of its own, its accessibility label is its text,
on both platforms. Thus an icon-only button labelled `Close` matches
`on_text = "Close"`. An image (Android `ImageView`, iOS `image`) is the
exception: its label describes a picture, so it is never text. In a webview,
`text` is the DOM text only.

### Prefer visible `text`; use `accessibility_label` sparingly

Default to `on_text` (or a positional/`contains` selector). Use
`on_accessibility_label` only:

1. **When the test is about the accessibility label itself** — e.g.
   validating screen-reader semantics / a11y compliance. Here the label *is* the
   thing under test.
2. **To navigate to the screen under test, when the tapped element is not what
   the step checks** — e.g. opening a menu by its stable `menu-toggle` id.

Outside those cases, an `accessibility_label` selector tests something the user
never perceives, and silently passes even if the visible text is wrong. When in
doubt, use `text`.
