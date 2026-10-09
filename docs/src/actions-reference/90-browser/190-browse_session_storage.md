### `browse_set_session_storage` / `browse_get_session_storage` — Session storage

```toml
{ action = "browse_set_session_storage", key = "step", value = "2" }
{ action = "browse_get_session_storage", key = "step", save_to = "step" }
```

Identical to the local-storage pair, against `sessionStorage`.

Storage and cookies need a real origin: a page reached by `browse_navigate` has
one, but `about:blank` doesn't, and both will fail there.
