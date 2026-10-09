### Timeout Multipliers

Each action has a built-in multiplier applied to the base timeout (`step_timeout`, default 5000ms). Per-step `timeout` always overrides.

| Multiplier | Timeout (at 5s base) | Actions |
|------------|---------------------|---------|
| 1x | 5s | `screenshot`, `add_media`, `fail`, `load_fixture`, `push_notification`, `clear_data`, `press`, `set_dark_mode`, `set_location`, `hide_keyboard` |
| 2x | 10s | `tap`, `double_tap`, `backspace`, `clear_text`, `long_press`, `swipe`, `pinch`, `gesture`, `rotate`, `type`, `assert_visible`, `assert_not_visible`, `read`, `assert_alert`, `accept_alert`, `dismiss_alert`, and any action not listed here |
| 4x | 20s | `bash`, `run` |
| 5x | 25s | `launch`, `stop` |
| 6x | 30s | `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http`, `open_link`, `create_inbox` |
| 8x | 40s | `scroll` (12x with `within`) |
| 48x | 240s | `await_email` |

The `auto_scroll = true` option sets 8x on any action (12x with `within`).

Actions that take time by themselves (`long_press`, `swipe` through 3+ points, `gesture`, `rotate`, `type`, `backspace`, `clear_text`) get at least that time plus 2s: `max(multiplied, duration + 2s)`. For `type` and `backspace` the time is 500ms per character.
