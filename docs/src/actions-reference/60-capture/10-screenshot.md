### `screenshot` — Take screenshot

Capture the screen. With `path`, save it there; a relative path resolves from the directory where you run golem. Without `path`, the image is captured but not saved.

```toml
{ action = "screenshot" }
{ action = "screenshot", path = "/tmp/dark-mode.png" }
```
