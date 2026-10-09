### `load_fixture` — Load fixture data

Load variables from a TOML file in `__fixtures__/` (a `[vars]` table). See
[reuse comparison](test-structure.md#reuse-subflow-vs-mixin-vs-fixture).

```toml
{ action = "load_fixture", fixture = "users", as = "test_user" }
# Access as ${test_user.email}, ${test_user.name}, etc.
```
