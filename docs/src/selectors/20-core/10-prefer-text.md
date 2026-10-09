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
