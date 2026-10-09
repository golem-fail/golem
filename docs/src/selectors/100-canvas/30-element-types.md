### Element types

`element_type`, and the `button` trait that reads it, are not reliable on a
canvas UI:

- **Android (Compose):** a `Text` is a `TextView`, and every other node is a
  `View`. A Material `Button` adds a separate `Button` node. That node has no text
  or label and is not clickable. The clickable node is the parent `View`, and the
  button's text is a child `TextView`.
- **iOS (Compose Multiplatform):** nodes get real types (`button`, `text`,
  `other`). But the `button` node's text is the merged string, for example
  `"Increment, +"`, and the visible `+` is a child `text` node.
- **Flutter:** a button is a `Button` (Android) or `button` (iOS) node that
  holds its own text. A `Text` is a plain `View` on Android and `text` on iOS.
  An `Icon` with a `semanticLabel` looks the same as a `Text` on both
  platforms. Only `Semantics(image: true)` and `Image` give an image node
  (`ImageView` / `image`).

As a result, on Compose `{ text = "+", traits = ["button"] }` matches nothing on
either platform. Select by text alone: `{ action = "tap", on_text = "+" }`.
