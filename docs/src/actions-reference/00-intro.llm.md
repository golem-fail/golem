# Actions

A step is one TOML inline table on one line. It is the same text as a step in a flow file:

```toml
{ action = "tap", on_text = "Sign in" }
```

Target an element with a selector:

- `on_text = "Sign in"`: the text a user reads. Glob (`*`, `?`), case-insensitive, full string: `"Item *"`. Prefer it.
- `on_accessibility_label = "login"`: the accessibility label or id. Use it only to test that label (screen readers), or when the element has no text.
- `on_below`, `on_above`, `on_right_of`, `on_left_of`: near another element. `on_index` picks one of several matches, from 0.
- A group: `on = { text = "OK", below = "Confirm?" }`. help("selectors") has the rest.

Options on any step: `timeout` (ms), `auto_scroll = true` (scroll to find the element), `if_fail = "warn"` or `"ignore"`, `retry` (count), `save_to` (a variable). help("flow", "step/options") lists them all.

A value can use a variable, `${name}`, or fake data, `${fake:email}` (help("fake")).

The common actions:

```toml
{ action = "tap", on_text = "Sign in" }
{ action = "type", on_text = "Email", input = "${fake:email}" }
{ action = "assert_visible", on_text = "Welcome*" }
{ action = "assert_not_visible", on_text = "Loading" }
{ action = "scroll", to = { text = "Terms" } }
{ action = "read", on_right_of = "Total:", save_to = "total" }
```

help("act", action) gives one action's fields and examples. help("act", group) lists a group.
