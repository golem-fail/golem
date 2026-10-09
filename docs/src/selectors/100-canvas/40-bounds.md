### Bounds and visibility

Every Compose and Flutter node carries `bounds` and `visible_bounds` on both
platforms. golem's viewport filter drops off-screen nodes, and `scroll` brings
them into view. Compose has two differences between the platforms:

- **Android** leaves off-screen Compose nodes out of the tree.
- **iOS** keeps every node of a non-lazy layout (for example a `Column` with
  `verticalScroll`) in the tree, with `visible_bounds` equal to `bounds`. That
  is, Compose Multiplatform does not clip `visible_bounds` to the scroll
  container.

A lazy Flutter `ListView` leaves its off-screen items out of the Android tree.
On iOS it keeps them, with a zero-size frame at the origin.

`assert_not_visible` searches the full tree, not the visible tree. Thus on iOS it
treats these nodes as present and waits until its timeout: an off-screen node
of a non-lazy Compose layout, and an off-screen item of a Flutter `ListView`. A
Compose lazy layout (`LazyColumn`) disposes of off-screen items, so
`assert_not_visible` works there on both platforms.
