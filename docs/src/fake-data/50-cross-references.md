## Cross-references
<!-- summary: A generator argument can read an earlier variable -->

Later generators can reference earlier variables:

```toml
[flow.vars]
addr   = "${fake:address(country=JP)}"
phone  = "${fake:phone(country=${addr.country_code})}"
person = "${fake:person(country=${addr.country_code})}"
```
