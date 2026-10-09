### phone

`${fake:phone}` → a plausibly-formatted phone number.

| Param | Effect |
|-------|--------|
| `country` | ISO code; uses that country's dialing format — `fake:phone(country=JP)` → `+81-…`. Unset → a random country's format. An **unrecognised** code is an error (a typo shouldn't silently yield another country) |
| `format` | Explicit template where `#` becomes a random digit — `fake:phone(format=+1 (###) ###-####)`. Takes precedence over `country` |

Chain it off an address to keep them consistent:
`phone = "${fake:phone(country=${addr.country_code})}"`.
