### Step Options

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | per-action | Max wait in ms. Overrides computed default. |
| `auto_scroll` | `false` | Scroll page to find element |
| `max_scrolls` | — | Limit scroll attempts |
| `keep_keyboard` | `false` | Leave the soft keyboard up — skips both the pre-tap dismissal and the resolver's occlusion recovery |
| `if_fail` | `"error"` | `"error"` (fail flow), `"warn"` (log + continue), `"ignore"` (silent continue) |
| `retry` | `0` | Retry count on failure |
| `retry_delay` | `1000` | Delay between retries (ms) |
| `save_to` | — | Save result to a variable |
| `app` | — | Target a specific app (for multi-app flows) |
