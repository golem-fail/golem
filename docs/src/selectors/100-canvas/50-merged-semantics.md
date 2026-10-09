### Merged semantics

`Modifier.semantics(mergeDescendants = true)`, `Modifier.clickable` and Material
components merge their children into one accessibility node. Flutter's
`MergeSemantics` does the same, but the two frameworks report the result
differently:

| | Compose, Android | Compose, iOS | Flutter, both platforms |
|---|---|---|---|
| Merged container | No text | Text is the children's text, joined: `"Click A, Click B"` | Text is the children's text, joined by a line break: `"Merged A\nMerged B"` |
| Child text nodes | Present | Present | **Absent** |

On Compose, child text nodes stay in the tree on both platforms. Thus these
selectors work on both:

- `on_text` on a child: `{ action = "tap", on_text = "Click B" }` taps inside the
  clickable container.
- `below` / `above` / `left_of` / `right_of` anchored on a child's text.

`inside` and `contains` anchored on the **container's** text work only on iOS,
because the Android container has no text. Anchor on a child's text, or on a
heading, instead.

On Flutter, a merged child has no node of its own. Select the container by its
joined text, with a glob: `{ action = "tap", on_text = "Merged A*" }`. `inside`
and `contains` are geometric, so a coarse tree does not change them.
