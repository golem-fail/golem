### Timeout Multipliers

Each action has a built-in multiplier applied to the base timeout (`step_timeout`, default 5000ms). Per-step `timeout` always overrides. `auto_scroll = true` forces 6x minimum.

| Multiplier | Timeout (at 5s base) | Actions |
|------------|---------------------|---------|
| 1x | 5s | `tap`, `double_tap`, `backspace`, `long_press`, `swipe`, `pinch`, `gesture`, `press`, `rotate`, `screenshot`, `hide_keyboard`, device controls |
| 2x | 10s | `type`, `assert_visible`, `assert_not_visible`, `read`, alerts |
| 3x | 15s | `launch`, `stop` |
| 4x | 20s | `bash`, `run` |
| 6x | 30s | `scroll`, `auto_scroll`, `*_http`, `open_link` |
| 48x | 240s | `await_email` |

Actions with intrinsic duration (`long_press`, `type`, `rotate`, `gesture`) auto-extend: `max(multiplied, duration + 2s)`. For `type`, duration is ~200ms per character.
