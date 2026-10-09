## Variables

Set variables from the CLI, flow metadata, data rows, `read` actions, or fixtures:

```bash
golem run flows/login.test.toml --var EMAIL=test@example.com
```

```toml
[flow.vars]
base_url = "https://staging.example.com"

[[block]]
steps = [
  { action = "read", on_right_of = "Status:", save_to = "current_status" },
  { action = "bash", run = "echo ${current_status}", save_to = "result" },
]
```
