### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

**Supported buttons (platform-specific):**

| `button`      | Android | iOS |
|---------------|---------|-----|
| `home`        | ✓       | ✓   |
| `back`        | ✓       | —   |
| `volume_up`   | ✓       | —   |
| `volume_down` | ✓       | —   |

An unsupported button fails the step.
