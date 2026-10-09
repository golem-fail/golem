### address

`${fake:address}` returns a coherent address for one place — all fields come from
the same city, so they stay consistent. There are no standalone city, postcode or
street generators: read the field you need (`${fake:address.city}`).

| Param | Effect |
|-------|--------|
| `country` | ISO 3166-1 alpha-2 code, case-insensitive. Unset → a random country. An unrecognised code is an error. Supported: AE, AU, BE, BR, CA, CN, DE, EG, ES, FR, GB, IE, IL, IN, JP, KR, LT, MX, NL, NZ, PL, RU, SE, SG, TH, US, ZA. |
| `state` | Limit to one state, by its native or romanised name (`東京` or `Tokyo`), case-insensitive. |
| `region` | Limit to the states tagged with this region, case-insensitive — e.g. `Kansai` / `Kanto` (JP), `New England` (US), `Scotland` (GB). The tags for each country are the `region_tags` in `data/geo/<code>.json`. |

`state` and `region` need `country`; without it they are ignored. A filter that
matches no state is an error.

The text fields default to the place's **native script**; an `ascii` sub-object
carries the romanised forms — exactly like `${fake:person}` (native default,
`.ascii` branch). Romanisations are ASCII folds of the native, never English
exonyms (`Bayern`, not "Bavaria").

#### Fields

| Field | `${fake:address}` (native) | `${fake:address.ascii.*}` |
|-------|---------|---------|
| `street` | 北一条西５ | 5 Kita 1-jo Nishi |
| `city` | 札幌市 | Sapporo |
| `state` | 北海道 | Hokkaido |
| `postcode` | 060-0001 | 060-0001 |
| `country` | 日本 | Nihon |
| `country_code` | JP | *(top-level only)* |
| `lat` | 43.0621 | *(top-level only)* |
| `lon` | 141.3544 | *(top-level only)* |

`country_code` / `lat` / `lon` are script-neutral and appear only at the top
level (not in `ascii`). The native and ascii streets share the same house
number, rendered in the script's own numerals (full-width for JP). `lat` / `lon`
are **approximate** — the chosen city's centre, not the street point. A Latin
country (e.g. GB) reads identically in both: native `42 Baker Street` ==
`ascii.street`.
