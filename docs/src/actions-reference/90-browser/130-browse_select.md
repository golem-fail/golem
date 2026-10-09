### `browse_select` — Choose an option in a `<select>`

```toml
{ action = "browse_select", selector = "#status", value = "fulfilled" }
{ action = "browse_select", selector = "#status", text = "Fulfilled" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `value` | — | Match the option's `value` attribute |
| `text` | — | Match the option's visible label instead. Give one or the other, not both |

Fires `input` and `change` the way a user's choice would, so a framework
listening for them sees the update.
