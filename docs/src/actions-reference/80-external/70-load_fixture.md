### `load_fixture` — Load fixture data

Load variables from a TOML file in `__fixtures__/` (a `[vars]` table). See
[reuse comparison](test-structure.md#reuse-subflow-vs-mixin-vs-fixture).

```toml
{ action = "load_fixture", fixture = "users", as = "test_user" }
# Access as ${test_user.email}, ${test_user.name}, etc.
```

| Field | Description |
|-------|-------------|
| `fixture` | Fixture name: `__fixtures__/<fixture>.toml`, looked up from the flow's directory up to the project root (required) |
| `as` | Variable name the fixture's vars are stored under (required) |
