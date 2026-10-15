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

A root `secret` list names the keys of `[vars]` that golem masks in all output. A name that is not a key of `[vars]` fails the fixture, so a typo cannot leave a value unmasked. See [Secrets](test-structure.md#secrets).

```toml
# __fixtures__/qa.toml
secret = ["password"]

[vars]
email = "qa@example.test"
password = "staging-password"
```
