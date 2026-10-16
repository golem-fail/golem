### Branching
<!-- summary: Arms race on the screen; the first that matches goes to its block, else the else or fail -->

A block's `branch` says where the flow goes after the block's steps. Each arm has a condition and a `goto`:

```toml
[[block]]
name = "start"
steps = [{ action = "launch", app = "app" }]
branch = [
  { if_visible = "Allow notifications", goto = "permissions" },
  { if_visible = "Home", goto = "home" },
]
```

The app opens on either screen. golem waits for the first one to show, and goes to its block as soon as it shows.

**The arms race.** golem checks the arms after the block's steps finish (for a `for_each` block, after the last row; for a `run_flow` block, after the child flow returns). It first waits for the screen to settle, then reads one tree on each poll and checks each arm on it, in order. The first arm that matches wins.

- `if_visible` and `if_not_visible` wait for their condition up to the timeout of `assert_visible`. Set `timeout` (in ms) on the arm to change that. `timeout = 0` checks once.
- `if_var` arms are checked once, at once. A `timeout` on one is an error.

| Condition | Matches when |
|---|---|
| `if_visible = "…"` | an element with this text (glob) is on screen |
| `if_not_visible = "…"` | no element with this text is on screen |
| `if_var = "x"`, `equals = "…"` | the variable equals the string exactly |
| `if_var = "x"`, `matches = "…"` | the variable matches the glob pattern |
| `if_var = "x"`, `gte = N` | the variable, read as an integer, is ≥ `N` (an integer). A non-integer value never matches. |
| (no condition) | always: this arm is the else |

`if_var` without `equals`, `matches` or `gte` never matches.

**When no arm matches,** the flow takes the else: the last arm, if it has no condition, or the block's `next`. Without an else, the flow fails with `EF406`. The failure names the block, each arm and why it did not match, and the texts on the screen. A block with a branch never goes on to the next block in the file.

```toml
[[block]]
name = "pay"
steps = [{ action = "tap", on_text = "Pay" }]
next = "receipt"                                      # the else
branch = [{ if_visible = "Error", timeout = 0, goto = "retry" }]
```

golem checks the arms when it reads the flow:

- An arm with no condition that is not the last arm is an error: the arms after it never run.
- An arm with no condition and `next` on the same block is an error: both are the else.
- Two arms with the same condition and timeout give a warning: the second never wins.
