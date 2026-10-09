## All generators

Everything `${fake:…}` supports, at a glance:

| Generator | Returns | Summary |
|-----------|---------|---------|
| `${fake:email}` | scalar | Random email address |
| `${fake:password}` | scalar | Random password |
| `${fake:uuid}` | scalar | UUID v4 |
| `${fake:number}` | scalar | Random integer |
| `${fake:one_of}` | scalar | Random pick from a caller-supplied set |
| `${fake:sentence}` | scalar | One-sentence filler; `language=` (en/fr/ja/ar), lorem default |
| `${fake:phone}` | scalar | Country-formatted phone number |
| `${fake:person}` | object | Names across scripts: `.given` / `.family` / `.reading` / `.ascii` / per-script branches |
| `${fake:address}` | object | `.street` / `.city` / `.state` / `.postcode` / `.country` / `.country_code` / `.lat` / `.lon` |
| `${fake:credit_card}` | object | Luhn-valid card: `.number` / `.expiry` / `.cvv` / `.brand` / `.status` |
| `${fake:timestamp}` | object | Date/time: `.datetime` / `.date` / `.time` / `.year` / `.month` / `.day`; window + age params |

Specifics below: [simple generators](#simple-generators) ·
[person](#person) · [address](#address) ·
[credit_card](#credit_card).
