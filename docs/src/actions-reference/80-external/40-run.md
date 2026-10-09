### `run` — Run project script

Execute a script relative to the project root or flow directory. Rejects path traversal (`..`).

```toml
{ action = "run", script = "/scripts/seed_db.sh" }
{ action = "run", script = "/scripts/setup.sh", args = ["staging", "verbose"], save_to = "output" }
```

Leading `/` = relative to project root. No leading `/` = relative to flow file directory.

The script runs directly, not through a shell, so it must be executable (`chmod +x`) and start with a shebang. A non-zero exit code fails the step and reports stderr. `save_to` stores an object: `${output.stdout}` (trimmed) and `${output.exit_code}`.
