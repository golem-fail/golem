## Canvas-rendered UI (Compose, Compose Multiplatform, Flutter)

Compose, Compose Multiplatform and Flutter copy their semantics tree into the platform accessibility tree, so golem sees one node per widget. Text, relational and containment selectors work on these nodes.

- **Select by visible text.** It is the one path that works the same on Android and iOS.
- **Identifiers.** A Compose `Modifier.testTag` or a Flutter `Semantics(identifier:)` is matched by `accessibility_label`. On Android, a Compose `testTag` reaches golem only with `testTagsAsResourceId = true` on an ancestor. Flutter needs 3.19 or later; Compose Multiplatform on iOS needs 1.8.0 or later.
- **`contentDescription` on iOS.** It merges into the element's text (`"Increment, +"`). On a `Text` it replaces the visible text, so do not put one on a `Text` whose value a step checks.
- **Element types.** On Compose, element types are not reliable, so neither is the `button` trait: `{ text = "+", traits = ["button"] }` matches nothing. Use `{ action = "tap", on_text = "+" }`.
- **Scrolled-out nodes on iOS.** A non-lazy Compose layout (a `Column` with `verticalScroll`) and a Flutter `ListView` keep off-screen nodes in the iOS tree. The viewport filter drops them, so `assert_not_visible` treats them as absent. A non-lazy Compose node hidden by a scroll container smaller than the screen keeps its full `visible_bounds`, so golem counts it as on screen.
- **Merged semantics.** On Compose, child text nodes stay in the tree: target a child's text (`on_text = "Click B"`); anchor `inside`/`contains` on a child's text, not the container's. On Flutter, `MergeSemantics` removes the children and the container's text is the children's text joined by a line break: select it with a glob (`on_text = "Merged A*"`).
- **Flutter custom widgets** (a `GestureDetector` on a `Container`, a `CustomPaint`) are invisible to golem until wrapped in `Semantics`.
