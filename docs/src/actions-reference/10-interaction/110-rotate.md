### `rotate` — Rotate gesture

A two-finger **rotation gesture** centered on an element (or screen). `rotate` is a multi-touch gesture, **not** a device-orientation change — programmatic device orientation is [unsupported](unsupported.md).

Two fingers orbit a center point — resolved from an element selector, or from explicit `x` / `y` coordinates.

```toml
{ action = "rotate", on_text = "Map", rotation = 90.0 }    # rotate 90° clockwise
{ action = "rotate", on_text = "Map", rotation = -45.0 }   # 45° counter-clockwise
```

| Field | Default | Description |
|-------|---------|-------------|
| `rotation` | — (required) | Degrees to rotate. Positive = clockwise, negative = counter-clockwise. |
| `velocity` | `180.0` | Rotation speed in degrees per second |
