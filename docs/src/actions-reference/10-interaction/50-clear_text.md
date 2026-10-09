### `clear_text` — Empty a field

Empties the **currently focused** text field in one step, whatever its length.
Like `backspace` it takes **no selector** — `type` or `tap` the field first.
Use it when the field's contents aren't known up front (a value carried over
from a previous run, a prefilled form); reach for `backspace` when you want to
remove a specific number of characters.

```toml
{ action = "tap", on_text = "Email" },
{ action = "clear_text" },
{ action = "type", input = "me@example.com" }
```

Takes no fields.

golem reads the focused field's length from the hierarchy and deletes exactly
that many characters, so no companion-side "select all" is involved. Two
consequences worth knowing:

- **The caret must be at the end.** Deletes only remove what is behind the
  caret, so a caret left mid-field can't reach the tail. golem detects this and
  fails the step telling you to re-focus, rather than silently half-clearing.
  `type` leaves the caret at the end; a `tap` places it where you tapped.
- **A field whose contents exactly equal its placeholder reads as empty** and is
  left alone. An empty field reports its placeholder as the text the user sees,
  and the two cases are indistinguishable on the wire.
