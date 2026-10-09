### Flutter

Flutter creates semantics for its standard widgets (`Text`, `Icon` with a
`semanticLabel`, Material buttons, text fields). A custom widget, for example a
`GestureDetector` on a `Container` or a `CustomPaint`, exposes nothing until you
wrap it in `Semantics`. Thus semantics are opt-in for each custom widget. This
is a bigger task than on Compose, where most interactive modifiers add
semantics.
