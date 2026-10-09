### `load_mixin` — Inline a reusable step sequence

Inline the steps from a mixin file in `__mixins__/` (a `[[step]]`-only file) into
the current block. Pass per-call values via `vars`, referenced as `${…}` inside
the mixin. See [reuse comparison](test-structure.md#reuse-subflow-vs-mixin-vs-fixture).

```toml
# __mixins__/launch_and_wait.toml
[[step]]
action = "launch"
app = "${app_bundle}"
[[step]]
action = "assert_visible"
on_text = "${wait_element}"
```

```toml
{ action = "load_mixin", mixin = "launch_and_wait", vars = { app_bundle = "app", wait_element = "Submit" } }
```

A mixin is a step fragment that runs inside the caller's block; for reusing a
whole scenario as a child, use a [subflow](test-structure.md#subflow) instead.
