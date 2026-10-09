### `bash` — Run shell command

Execute a command via `sh -c`. A non-zero exit code fails the step and reports the command's stderr. `save_to` stores stdout, trimmed.

```toml
{ action = "bash", run = "curl -s https://api.example.com/reset" }
{ action = "bash", run = "echo $ENV_VAR", save_to = "result" }
```
