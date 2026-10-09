## Canvas-rendered UI (Compose, Compose Multiplatform, Flutter)

Jetpack Compose, Compose Multiplatform and Flutter draw the whole UI into one
platform view. They do not appear to golem as one opaque leaf, the way a WebView
without enrichment does. Each framework copies its semantics tree into the
platform accessibility tree, so golem sees one node for each widget. Text
selectors, relational selectors and viewport filtering work on these nodes.

What changes is *which* annotation reaches golem, and how coarse the tree is.
The facts below come from Jetpack Compose (`test-app-b`), Compose Multiplatform
1.11 (`test-app-k`) and Flutter 3.47 (`test-app-d`), on Android API 36 and
iOS 26.
