### `swipe` — Swipe gesture

`swipe` is the **raw** gesture primitive — one direction-based swipe or a path-based gesture defined by `start` / `end` (and optional `points` for 3+ point paths). Use `scroll` instead when you want golem to *keep* swiping until an element appears.

```toml
# Direction-based — single swipe from a sensible default origin
{ action = "swipe", direction = "down" }
{ action = "swipe", direction = "left" }

# Path-based with selectors — start and end resolve to element centres
{ action = "swipe", start = { text = "Slider" }, end = { text = "Max" }, duration = 500 }

# Anchored to a container (no `within` for swipe — use `start` / `end`)
{ action = "swipe",
  start = { below = "Scroll List" },
  end   = { below = "Scroll List", y = "30%" } }
```

| Field | Description |
|-------|-------------|
| `direction` | `"up"`, `"down"`, `"left"`, `"right"` |
| `start` | Start position (SelectorGroup: text / accessibility_label / below / above + optional x / y offsets) |
| `end` | End position (SelectorGroup) |
| `points` | Array of intermediate points for complex paths |
| `duration` | Gesture duration in ms |

> **Note:** `within` is **not** consumed by `swipe` — only by `scroll` and by any step with `auto_scroll = true`. Use `start` / `end` to anchor a swipe inside a container. A `within` set on a swipe (or other unsupported action) emits a `[lint]` warning at plan time; a future `--validate` mode will reject it as an error.
