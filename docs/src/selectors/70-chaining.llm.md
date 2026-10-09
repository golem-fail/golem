## Nesting and chaining

- An anchor (`below`, `above`, `left_of`, `right_of`, `contains`, `inside`) is bare text or a full selector group, and a group may have its own anchors: `{ text = "Left", below = { text = "Nested Layout", traits = ["has_text"] } }`. The anchor is the group's first match, and it must be on screen.
- Every key in a group is combined with AND.
- Survivors sort: containment tightest-first, then nearest to the directional anchor, then tree order. `index` (0-based) picks from that list.
- golem does not guess between equal candidates: a tie resolves by tree order. To pick a specific one, add `index` or another predicate.
