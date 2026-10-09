## Determinism and seeds

- The same seed gives the same `${fake:…}` values, so assertions on generated values replay exactly.
- Set it with `golem run <flow> --seed <N>`. Without `--seed`, values are fresh each run, and the report shows the seed used; pass it back with `--seed` to replay that run. In an MCP session: `session_open(seed = <N>)`. A `[flow] seed` in the file has no effect.
- Dates (`fake:timestamp`, card expiry) are relative to an anchor carried in the seed: a run without `--seed` anchors at the current time; a small seed like `--seed 42` anchors at 2020.
