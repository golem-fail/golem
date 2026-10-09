### Branching

Control flow between blocks with conditions:

```toml
[[block]]
name = "check_state"
steps = [
  { action = "assert_visible", on_text = "Welcome", if_fail = "ignore" },
]

[[block.branch]]
if_visible = "Dashboard"
goto = "already_logged_in"

[[block.branch]]
goto = "login_required"            # Unconditional fallback
```

Branches are checked after the block's steps finish (for a `for_each` block, after the last row; for a `run_flow` block, after the child flow returns). They are checked in order, and the first match wins. If none matches, the flow continues with the next block in document order.

| Condition | Matches when |
|---|---|
| `if_visible = "…"` | an element with this text (glob) is on screen |
| `if_not_visible = "…"` | no element with this text is on screen |
| `if_var = "x"`, `equals = "…"` | the variable equals the string exactly |
| `if_var = "x"`, `matches = "…"` | the variable matches the glob pattern |
| `if_var = "x"`, `gte = N` | the variable, read as an integer, is ≥ `N` (an integer). A non-integer value never matches. |
| (no condition) | always |

`if_var` without `equals`, `matches` or `gte` never matches. A block with any `[[block.branch]]` ignores its `next`, even when no branch matches — put an unconditional `goto` last instead.
