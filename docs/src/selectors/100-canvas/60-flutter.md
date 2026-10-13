### Flutter
<!-- summary: Wrap a custom Flutter widget in Semantics so golem sees it -->

A Flutter custom widget (for example a `GestureDetector` on a `Container`, or a
`CustomPaint`) is invisible to golem until you wrap it in `Semantics`.
