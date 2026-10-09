### Step Options

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | per-action | Max wait in ms. Overrides computed default. |
| `auto_scroll` | `false` | Scroll page to find element |
| `max_scrolls` | — | Limit scroll attempts |
| `within` | — | With `scroll` or `auto_scroll`: scroll only inside the element this selector matches — see [`within`](selectors.md#within-scoping-a-scroll) |
| `keep_keyboard` | `false` | Leave the soft keyboard up: golem does not dismiss it before a tap or when it hides the target |
| `if_fail` | `"error"` | `"error"` (fail flow), `"warn"` (log + continue), `"ignore"` (silent continue) |
| `retry` | `0` | Retry count on failure |
| `retry_delay` | `1000` | Delay between retries (ms) |
| `save_to` | — | Save result to a variable |
| `app` | — | Target a specific app (for multi-app flows) |
