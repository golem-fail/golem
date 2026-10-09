### `bash` — Run shell command

Execute a command via `sh -c`. Fails if exit code is non-zero.

```toml
{ action = "bash", run = "curl -s https://api.example.com/reset" }
{ action = "bash", run = "echo $ENV_VAR", save_to = "result" }
```
