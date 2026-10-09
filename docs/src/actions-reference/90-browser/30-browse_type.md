### `browse_type` — Type into a field

```toml
{ action = "browse_type", selector = "#email", text = "${inbox.address}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | — | The value to type. `input` is accepted as a synonym, matching the mobile `type` action |

The field is clicked first, so the keystrokes land in it rather than wherever
focus happened to be.
