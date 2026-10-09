# Fake Data Generators

*Deterministic, seed-stable test data.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) · [Selectors](selectors.md)

Generate realistic test data with `${fake:…}`. A generator works anywhere `${…}`
interpolation does — in a variable declaration or inline in a step field:

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
card  = "${fake:credit_card(brand=visa)}"

[[block]]
steps = [
  { action = "type", on_text = "Email", input = "${fake:email}" },
  { action = "type", on_text = "City",  input = "${addr.city}" },
]
```

A **scalar** generator (e.g. `email`) resolves straight to a string. An
**object** generator (`person`, `address`, `credit_card`, `timestamp`) exposes several fields
— read one with dot notation (`${user.given}`, `${addr.city}`, `${card.number}`);
using the bare object where a string is expected is an error.

<!-- toc -->
