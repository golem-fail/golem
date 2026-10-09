### `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests

```toml
{ action = "get_http", url = "https://api.example.com/status", save_to = "response" }
{ action = "post_http", url = "https://api.example.com/reset", body = "{\"force\": true}" }
{ action = "get_http", url = "https://api.example.com/data", headers = { Authorization = "Bearer ${token}" } }
```

Fails on non-2xx status codes.
