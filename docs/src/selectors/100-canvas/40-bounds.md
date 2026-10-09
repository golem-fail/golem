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

Both kinds of node fall outside the viewport, so `assert_not_visible` treats
them as absent on both platforms, as a user would. One caveat remains on iOS:
a non-lazy Compose node that its scroll container hides but that is still
inside the screen keeps its full `visible_bounds`, so golem counts it as on
screen.
