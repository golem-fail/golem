### `pinch` — Pinch zoom gesture

Two-finger pinch centered on an element or coordinates.

```toml
{ action = "pinch", scale = 2.0, duration = 500 }     # Zoom in
{ action = "pinch", scale = 0.5, duration = 500 }     # Zoom out
{ action = "pinch", on_text = "Map", scale = 2.0 }    # Centered on an element
{ action = "pinch", x = "50%", y = 300, scale = 0.5 } # Centered on a point
```

| Field | Default | Description |
|-------|---------|-------------|
| `scale` | — | `>1.0` = zoom in, `<1.0` = zoom out |
| `velocity` | `5.0` | Scale factor per second |
| center | screen centre | An element (`on_text`, `on_accessibility_label` or `on = { … }`), or `x` / `y` as pixels or `"N%"` of the screen. With an element, `x` / `y` are ignored. |
