### `swipe` — Swipe gesture

`swipe` is the **raw** gesture primitive — one direction-based swipe or a path-based gesture defined by `start` / `end` (and optional `points` for 3+ point paths). Use `scroll` instead when you want golem to *keep* swiping until an element appears.

```toml
# Direction-based — single swipe from a sensible default origin
{ action = "swipe", direction = "down" }
{ action = "swipe", direction = "left" }

# Path-based with selectors — start and end resolve to element centres
{ action = "swipe", start = { text = "Slider" }, end = { text = "Max" } }

# Anchored to a container: end is 30% of the element's height below its centre
{ action = "swipe",
  start = { below = "Scroll List" },
  end   = { below = "Scroll List", y = "30%" } }
```

| Field | Description |
|-------|-------------|
| `direction` | `"up"`, `"down"`, `"left"`, `"right"` |
| `start` | Start position: a selector group (`text` / `accessibility_label` / `below` / `above`) plus optional `x` / `y`. With an element, `x` / `y` offset from its centre: pixels, or `"N%"` of the element's size (`"50%"` = edge). Without an element, `x` / `y` are screen pixels or `"N%"` of the screen. |
| `end` | End position, same format as `start` |
| `points` | Array of intermediate points, same format as `start` |
| `duration` | Gesture duration in ms for a path of 3+ points (default `300`); a 2-point swipe ignores it |

`within` is ignored on swipe (lint warning); use `start` / `end`.
