### `backspace` — Delete characters

Deletes `count` characters from the **currently focused** text field. It
takes **no selector** — `type` or `tap` the field
first; the caret is left at the end of the text, so backspace removes from
there. A selector is rejected: a tap-to-focus would re-place the caret at the
tap point (mid-text on a filled field, deleting the wrong char), and there is
no reliable cross-platform way to move the caret to the end.

```toml
{ action = "type", on_text = "Email", input = "me@example.comm" },
{ action = "backspace", count = 1 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `count` | `1` | Number of characters to delete from the focused field |
