### `type` — Type text into an element

With a selector, taps the element to focus it, then types the `input`
string. No selector = type into the focused field.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
{ action = "type", input = " and more" }   # append to the focused field
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |
