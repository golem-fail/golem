### Which annotation reaches golem

| Annotation | Android | iOS |
|------------|---------|-----|
| Compose `contentDescription` | `accessibility_label`, on its own node, which has no other text, so the description is also that node's `text`. The visible text stays a separate text node. | Merged into the element's text. A button with text `+` and description `Increment` reads `"Increment, +"`. On a `Text`, the description **replaces** the visible text. |
| Compose `Modifier.testTag` | Identifier (`resource-id`), only with `testTagsAsResourceId = true` on an ancestor. | Identifier (`accessibilityIdentifier`), with no opt-in. |
| Flutter `Semantics(identifier:)` | Identifier (`resource-id`). | Identifier (`accessibilityIdentifier`). |
| Flutter widget text (`Text`, button text) | The label; golem reads it as `text` (see [What counts as text](#core-selectors)). | `text` |
| Visible text | `text` | `text` |

The `accessibility_label` selector matches the label or the identifier.

Visible text is the one path that works the same on both platforms. Use it (see
[Prefer visible `text`](#prefer-visible-text-use-accessibility_label-sparingly)). Do not put a
`contentDescription` on a `Text` whose value a step checks, because on iOS the
step then reads the description.
