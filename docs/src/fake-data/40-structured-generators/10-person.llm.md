### person

`${fake:person}` is an object of name parts. Read a field; the bare object is an error in a string.

- `country` is the **form's** country: it sets the script the form expects, not where the name comes from. A name in the country's script stays native; any other name falls back to ASCII (`country=JP`: `田中` stays, `Dupont` stays `Dupont`; `country=DE`: `Müller` stays, `André` becomes `Andre`).
- Names include characters that break naive validators: `O'Brien`, `Jean-Pierre`, `ß`.
- There is no full name. Build it the way the form wants: `"${p.given} ${p.family}"` (Western), `"${p.family}${p.given}"` (JP/KR/CN, no space).
- Every field is a string; a part that does not apply is `""`.

| Field | Value |
|-------|-------|
| `given` / `family` | What a local types into the form's given/family field |
| `reading.given` / `reading.family` | Reading (furigana): katakana for `country=JP`, hangul for `country=KR`; `""` elsewhere |
| `ascii.given` / `ascii.family` | Pure-ASCII romanisation |
| `<rep>.given` / `<rep>.family` | One representation, regardless of country: `native`, `ascii`, `kana`, `hiragana`, `katakana`, `hangul`, `cyrillic`, `hebrew`, `arabic`, `hanja` |

```toml
[flow.vars]
p = "${fake:person(country=JP)}"
# ${p.family} -> 田中   ${p.reading.given} -> ユキ   ${p.ascii.family} -> Tanaka
```

| Param | Sets | Example |
|-------|------|---------|
| `country` | A preset for `name`, `reading` and `local` | `country=JP` |
| `name` | The chain for `given`/`family`: the first non-empty representation wins | `name=[local, ascii]` |
| `reading` | The chain for `reading` | `reading=[katakana]` |
| `local` | The characters counted as native, for the `local` representation (a set, not a chain) | `local=[ascii, diacritics_fr]` |

- Precedence: an explicit `name`/`reading`/`local` beats the `country` preset, which beats the default (no country: `name=[native]`, no reading, `local` accepts everything).
- Representation tokens (`name`, `reading`): `native`, `local`, `ascii`, `kana`, `hiragana`, `katakana`, `hangul`, `cyrillic`, `hebrew`, `arabic`, `hanja`. `local` is the native name if every character is in the `local` set, else `""`.
- Repertoire tokens (`local`): `ascii`, `kanji`, `hiragana`, `katakana`, `hangul`, `hanzi`, `hanja`, `cyrillic`, `hebrew`, `arabic`, `devanagari`, `thai`, `diacritics_<lang>` for `de fr es pt it sv ga mi pl lt nl`. An unknown token is an error.
- Supported `country` codes (case-insensitive): AE, AU, BE, BR, CA, CN, DE, EG, ES, FR, GB, IE, IL, IN, JP, KR, LT, MX, NL, NZ, PL, RU, SE, SG, TH, US, ZA. Every preset's `name` is `[local, ascii]`. JP (`reading=[katakana]`) and KR (`reading=[hangul]`) also fill `reading`. An unlisted code is not an error; it behaves like no country.
