### `browse_set_local_storage` / `browse_get_local_storage` — Local storage

```toml
{ action = "browse_set_local_storage", key = "feature_flags", value = "{\"beta\": true}" }
{ action = "browse_get_local_storage", key = "session", save_to = "session" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `key` | — | Required |
| `value` | — | Required for the setter |

**Reading parses JSON objects.** Web apps keep structured state in storage as
JSON text, so an object nests and `${session.user.id}` works. Anything else — an
array, a number, plain text — stays the text it was. A key that isn't set fails
with `F404`.
