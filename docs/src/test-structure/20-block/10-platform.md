### Platform-Specific Blocks

Skip blocks that don't apply to the current device:

```toml
[[block]]
name = "android_back"
where = { os = "android" }
steps = [
  { action = "press", button = "back" },
]
```
