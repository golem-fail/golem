## Fake Data Generators

Generate realistic test data with `${fake:…}` — `email`, `password`, `uuid`,
`number`, `phone`, `city`, and the structured `person` / `address` /
`credit_card`. Values are random but valid; `--seed <N>` replays deterministically.

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
```

`${fake:person}` is country-aware and exposes each name part as a
`given` / `family` pair across scripts — `person.given` / `.family` (primary,
country-aware), `person.reading.*` (furigana/reading), `person.ascii.*` (Latin),
plus raw per-script branches like `person.katakana.*` and `person.hangul.*`.
There is no joined full name — a form decides order and separator.

**See [Fake Data Generators](fake-data.md) for the full reference**: every
simple generator, the structured generators, and the `person` representation /
chain / `country` model.
