### Platform-Specific Blocks

`where` runs a block only on devices that match it; on other devices the block is skipped:

```toml
[[block]]
name = "android_back"
where = { os = "android" }
steps = [
  { action = "press", button = "back" },
]
```

| Key | Matches |
|---|---|
| `os` | `"ios"` / `"android"` — the platform; any other value is an OS-version prefix (`"17"` matches 17.x) |
| `type` | `"phone"` / `"tablet"` |
| `physical` | `true` on a physical device, `false` on a simulator/emulator |

A device must match every key given. A skipped block's `next` and `[[block.branch]]` are not evaluated: the flow continues with the next block in document order.
