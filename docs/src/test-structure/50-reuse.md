## Reuse: Subflow vs Mixin vs Fixture

Three ways to share pieces across flows, by what they contain:

| Concept | File / location | Contains | Reused via | Use when |
|---|---|---|---|---|
| **flow** | `x.test.toml` | `[flow]` + `[[block]]` | — (top-level unit) | a complete scenario |
| **subflow** | `x.test.toml`, usually `explicit_only = true` | a full `[flow]` | `run_flow` on a `[[block]]`; `[block.save_to]` propagates results back | reusing a whole scenario as a child (e.g. `login`) |
| **mixin** | `__mixins__/x.toml` | `[[step]]` only (no flow/block/vars) | [`load_mixin`](actions-reference.md#load_mixin--inline-a-reusable-step-sequence) action; steps inline into the block, per-call `vars` | reusing a step fragment that runs inside the caller's block state |
| **fixture** | `__fixtures__/x.toml` | `[vars]` only | [`load_fixture`](actions-reference.md#load_fixture--load-fixture-data) action; access as `${alias.key}` | reusing test **data** |

`__mixins__/` and `__fixtures__/` are excluded from flow discovery, so their files never run as tests on their own.
