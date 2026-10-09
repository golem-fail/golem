## Core selectors

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
