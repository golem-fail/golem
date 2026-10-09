## Fake Data Generators

`${fake:…}` generates random but valid test data: `email`, `password`, `uuid`, `number`, `one_of`, `sentence`, `timestamp`, `phone`, and the structured `person`, `address` and `credit_card`. `--seed <N>` replays the same values.

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
```

**See [Fake Data Generators](fake-data.md)** for every generator, its parameters and fields.
