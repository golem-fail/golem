### `gesture` — Multi-touch gesture

Arbitrary multi-finger gesture with explicit paths.

```toml
[[block.steps]]
action = "gesture"
duration = 300

[[block.steps.fingers]]
points = [
  { x = 200, y = 400 },
  { x = 200, y = 200 },
]

[[block.steps.fingers]]
points = [
  { x = 200, y = 200 },
  { x = 200, y = 400 },
]
```

| Field | Default | Description |
|-------|---------|-------------|
| `fingers` | — | Array of finger paths, each with `points` (at least 2 per finger) |
| `duration` | `300` | Time (ms) each finger takes to travel its whole path |

A point is `x` / `y` screen coordinates (pixels or `"N%"` of the screen), or a
selector group (`text` / `accessibility_label` / `below` / `above`) plus
optional `x` / `y` offsets from the element's centre (pixels, or `"N%"` of the
element's size).
