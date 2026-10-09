### `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests

```toml
{ action = "get_http", url = "https://api.example.com/status", save_to = "response" }
{ action = "post_http", url = "https://api.example.com/reset", body = "{\"force\": true}" }
{ action = "get_http", url = "https://api.example.com/data", headers = { Authorization = "Bearer ${token}" } }
```

| Field | Description |
|-------|-------------|
| `url` | Request URL (required) |
| `body` | Request body, as a string |
| `headers` | Table of header name to string value |
| `save_to` | Variable to store the response body under, as a string |

A non-2xx status fails the step.
