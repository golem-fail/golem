### `fail` — Fail the flow immediately

```toml
{ action = "fail", message = "Unexpected state reached" }
{ action = "fail", message = "Bad total: ${order.total}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `message` | `"Flow failed (no message provided)"` | Failure reason shown in reports; supports inline `${…}` vars |

The only field `fail` uses is `message`. Useful in conditional branches to mark
unreachable paths.
