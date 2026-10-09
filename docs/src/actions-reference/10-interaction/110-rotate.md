### `rotate` — Rotate gesture

A two-finger **rotation gesture** centered on an element (or screen). `rotate` is a multi-touch gesture, **not** a device-orientation change — programmatic device orientation is [unsupported](unsupported.md).

Two fingers orbit a center point: an element (`on_text`, `on_accessibility_label` or `on = { … }`), or `x` / `y` as pixels or `"N%"` of the screen. Without either, the screen centre. With an element, `x` / `y` are ignored.

```toml
{ action = "rotate", on_text = "Map", rotation = 90.0 }    # rotate 90° clockwise
{ action = "rotate", on_text = "Map", rotation = -45.0 }   # 45° counter-clockwise
```

| Field | Default | Description |
|-------|---------|-------------|
| `rotation` | — (required) | Degrees to rotate. Positive = clockwise, negative = counter-clockwise. |
| `velocity` | `180.0` | Rotation speed in degrees per second |
