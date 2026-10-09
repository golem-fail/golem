### `type` — Type text into an element

With a selector, taps the element to focus it, then types the `input`
string. The selector is **optional**: with no selector, `type` sends the
keystrokes to the currently focused field without tapping — useful for
appending to the field the previous step left focused (the caret stays at
the end), or for apps that respond to keypresses outside a text input.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
{ action = "type", input = " and more" }   # append to the focused field
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |
