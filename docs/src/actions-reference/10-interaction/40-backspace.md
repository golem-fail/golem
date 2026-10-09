### `backspace` — Delete characters

Deletes `count` characters before the caret in the **currently focused** text
field. `type` or `tap` the field first; `type` leaves the caret at the end of
the text. A selector is an error.

```toml
{ action = "type", on_text = "Email", input = "me@example.comm" },
{ action = "backspace", count = 1 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `count` | `1` | Number of characters to delete from the focused field |
