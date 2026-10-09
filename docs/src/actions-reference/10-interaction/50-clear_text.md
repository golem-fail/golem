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

- **The caret must be at the end.** If it isn't, the step fails and tells you
  to re-focus the field. `type` leaves the caret at the end; a `tap` places it
  where you tapped.
- **A field whose contents exactly equal its placeholder reads as empty** and is
  left alone.
