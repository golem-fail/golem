# Fake Data Generators

Generate realistic test data with `${fake:…}`. Generators work anywhere `${…}`
interpolation does — in variable declarations and inline in step fields (e.g.
`input = "${fake:email}"`).

A whole-value declaration keeps an object (`card = "${fake:credit_card()}"`,
then `${card.number}`); used in a string or step field a generator must resolve
to a scalar — use `.field` for objects.

Generators produce random but valid values; use `--seed <N>` for deterministic
replay.

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
card  = "${fake:credit_card(brand=visa)}"
```

Access structured fields with dot notation: `${user.name}`, `${addr.city}`, `${card.number}`.

## Contents

- [Simple generators](#simple-generators)
- [Structured generators](#structured-generators)
  - [person — names across scripts](#person--names-across-scripts)
    - [The three fields you usually want](#the-three-fields-you-usually-want)
    - [How `name` is resolved: representations and chains](#how-name-is-resolved-representations-and-chains)
    - [Raw representation branches](#raw-representation-branches)
    - [Parameters](#person-parameters)
    - [Per-country behaviour](#per-country-behaviour)
  - [address](#address)
  - [credit_card](#credit_card)
- [Cross-references](#cross-references)

## Simple generators

| Generator | Output | Parameters |
|-----------|--------|------------|
| `${fake:email}` | `abc123@example.com` | `prefix`, `domain` |
| `${fake:password}` | Random password | `length` (default 12), `symbols` (default true) |
| `${fake:uuid}` | UUID v4 | — |
| `${fake:number}` | Random integer string | `min` (default 0), `max` (default 100) |
| `${fake:sentence}` | Simple English sentence | — |
| `${fake:timestamp}` | ISO 8601 within last year | — |
| `${fake:phone}` | Country-formatted phone | `country` (ISO code), `format` (`#` = digit) |
| `${fake:city}` | City name | `country`, `region` |
| `${fake:postcode}` | Postal code | `country` |
| `${fake:street}` | Street address | `country` |

For names, use `${fake:person}` (below): it is country-aware and exposes every
script explicitly.

## Structured generators

These return objects with multiple fields.

### person — names across scripts

`${fake:person}` draws a first and last name from a single global pool and makes
them available in many writing systems. The pool is intentionally diverse — it
spans Latin (with the full range of diacritics), Japanese, Korean, Chinese,
Cyrillic, Arabic, Hebrew, Devanagari and Thai, and deliberately includes the
characters that break naive form validators (apostrophes like `O'Brien`, hyphens
like `Jean-Pierre`, the German `ß`).

A name's origin is **not** tied to the `country` parameter — people move. The
`country` describes the **form being filled in**, which decides two things: the
name *order* (family-first for JP/CN/KR) and which script the form expects.

#### The three fields you usually want

A form rarely needs more than three name fields. These are resolved for you:

| Field | What it is |
|-------|------------|
| `person.name` (and `.first` / `.last`) | **Primary** — what a local would type into the main name field of a form in `country`. |
| `person.reading.name` (`.first` / `.last`) | **Reading / furigana** field, where the form has one (Japanese katakana, Korean hangul). Empty otherwise. |
| `person.ascii.name` (`.first` / `.last`) | **Latin** — a pure-ASCII romanisation, always safe for an ASCII-only field. |

The bare `person.first` / `.last` / `.name` are the same as `person.name.*` (the
primary field). `person` is names only — for an email or phone use the
dedicated `${fake:email}` / `${fake:phone(country=…)}` generators.

```toml
[flow.vars]
p = "${fake:person(country=JP)}"
# A Japanese name draws as kanji; a foreign name romanises on the JP form.
# ${p.name}          -> 田中 ゆき      (kanji, family-first)   OR  Dupont Jean
# ${p.reading.name}  -> タナカ ユキ     (katakana furigana)
# ${p.ascii.name}    -> Tanaka Yuki
```

Every `person.X.Y` field is **always a string** — never undefined. When a field
doesn't apply to a given person (e.g. the reading of a name on a form that has no
reading field) it is the empty string `""`.

#### How `name` is resolved: representations and chains

A *representation* is one way of writing a name part (its `native` form, an
`ascii` fold, a `katakana` reading, …). Each field is resolved through a
**fallback chain** of representations: the first one that yields a non-empty
value wins; if none do, the result is `""`.

The key representation is **`local`**: it returns the native name **iff every
character is acceptable for the country's script** (its *repertoire*), and `""`
otherwise. So the default primary chain `[local, ascii]` means:

- a name that fits the country's script is kept in its native form, but
- a name that doesn't (a foreigner's name on that form) falls through to the
  ASCII romanisation.

On a Japanese form (`local = [kanji, hiragana]`), `田中` is kept but `Dupont`
romanises; on a German form (`local = [ascii, diacritics_de]`), `Müller` is kept
(ü is German) but `André` becomes `Andre` (é is not).

#### Raw representation branches

Every representation is also exposed directly as `person.<rep>.{first,last,name}`,
regardless of country. Use these when you need a specific script explicitly. Any
of them may be empty for a given person.

| `<rep>` | Produced from |
|---------|---------------|
| `native` | the name as stored (its own script) |
| `ascii` | romanised Latin, diacritics folded — always ASCII-safe |
| `kana` | Japanese reading: hiragana for Japanese names, katakana for foreign |
| `katakana` | `kana` folded to katakana (always available) |
| `hiragana` | the reading when it is hiragana (a Japanese name); else `""` |
| `hangul` / `cyrillic` / `hebrew` / `arabic` | the name in that script — native if already so, else transcribed from a stored IPA reading |
| `hanja` | Korean Hanja, where the name has one; else `""` |

For the transcribed scripts (`hangul`/`cyrillic`/`hebrew`/`arabic`): if the name
is **already** in that script the native form is used verbatim (e.g.
`Cohen`→`כהן`, `Tariq`→`طارق`); otherwise it is a **consistent, loanword-style
approximation** from a stored IPA reading — not an authoritative spelling.

The full `name`/`reading` of a branch is its `first` and `last` joined in the
country's order; empty parts are dropped, so a mixed name (Japanese surname +
foreign given name) falls out naturally.

#### person parameters

Three things are configurable, each overridable independently:

| Parameter | Sets | Example |
|-----------|------|---------|
| `country` | a **preset bundle** — the `name`/`reading` chains, the `local` repertoire, and the name order, all at once | `country=JP` |
| `name` | the primary chain (overrides the country's) | `name=[local, ascii]` |
| `reading` | the reading chain | `reading=[katakana]` |
| `local` | the accepted repertoire for the `local` representation | `local=[kanji, hiragana]` |

Chain/list values use bracketed, comma-separated tokens: `name=[local, ascii]`.
**Precedence:** an explicit `name`/`reading`/`local` parameter wins over the
`country` preset, which wins over the built-in default (no country → `name` is
`[native]`, `reading` is empty, `local` accepts everything).

**Representation tokens** (for `name` / `reading`): `native`, `local`, `ascii`,
`kana`, `hiragana`, `katakana`, `hangul`, `cyrillic`, `hebrew`, `arabic`,
`hanja`.

**Repertoire tokens** (for `local`) are *named character sets*, not raw Unicode
scripts; the comma means **union** (a character is accepted if any listed
repertoire contains it):

- `ascii`, `kanji` (the JIS X 0208 kanji — narrower than Han, so simplified
  Chinese is rejected), `hiragana`, `katakana`, `hangul`, `hanzi`, `hanja`,
  `cyrillic`, `hebrew`, `arabic`, `devanagari`, `thai`
- `diacritics_<lang>` — a language's accented letters only (no ASCII), keyed by
  ISO 639-1 code: `diacritics_de fr es pt it sv ga (Irish) mi (Māori) pl lt nl`.

```toml
# A katakana-only field, regardless of the person:
furigana = "${fake:person(reading=[katakana]).reading.name}"

# Accept Latin names with French OR Portuguese accents, else romanise:
u = "${fake:person(local=[ascii, diacritics_fr, diacritics_pt])}"
```

#### Per-country behaviour

`country` presets are defined in the per-country `data/geo/*.json` files. A few:

| `country` | `local` repertoire | primary `name` | `reading` | order |
|-----------|--------------------|----------------|-----------|-------|
| JP | `kanji`, `hiragana` | `[local, ascii]` | `[katakana]` | family-first |
| KR | `hangul` | `[local, ascii]` | `[hangul]` | family-first |
| CN | `hanzi` | `[local, ascii]` | — | family-first |
| RU | `cyrillic` | `[local, ascii]` | — | given-first |
| TH | `thai` | `[local, ascii]` | — | given-first |
| IN | `devanagari` | `[local, ascii]` | — | given-first |
| AE / EG | `arabic` | `[local, ascii]` | — | given-first |
| IL | `hebrew` | `[local, ascii]` | — | given-first |
| DE | `ascii`, `diacritics_de` | `[local, ascii]` | — | given-first |
| FR / CA | `ascii`, `diacritics_fr` | `[local, ascii]` | — | given-first |
| ES / MX | `ascii`, `diacritics_es` | `[local, ascii]` | — | given-first |
| BR | `ascii`, `diacritics_pt` | `[local, ascii]` | — | given-first |
| SE | `ascii`, `diacritics_sv` | `[local, ascii]` | — | given-first |
| IE / NZ / PL / LT / NL | `ascii`, `diacritics_<lang>` | `[local, ascii]` | — | given-first |
| BE | `ascii` + French/Dutch/German accents | `[local, ascii]` | — | given-first |
| US / GB / AU / ZA / SG | `ascii` | `[local, ascii]` | — | given-first |
| (none) | accepts everything | `[native]` | — | given-first |

### address

`${fake:address}` — Parameters: `country`, `state`, `region`.

| Field | Example |
|-------|---------|
| `street` | 42 Baker Street |
| `city` | London |
| `state` | England |
| `postcode` | SW1A 1AA |
| `country` | United Kingdom |
| `country_code` | GB |

### credit_card

`${fake:credit_card}` — Generates Luhn-valid card numbers. Parameters: `brand`
(visa/mastercard/amex/discover), `provider` (stripe/adyen/square/etc.), `status`.

| Field | Example |
|-------|---------|
| `number` | 4532015112830366 |
| `expiry` | 03/28 |
| `cvv` | 123 |
| `brand` | visa |
| `status` | (empty if approved) |

Status options without provider: `approved`, `declined:invalid_number`,
`declined:expired`, `declined:invalid_cvv`, `threeds`. Provider-specific statuses
vary.

## Cross-references

Later generators can reference earlier variables:

```toml
[flow.vars]
addr   = "${fake:address(country=JP)}"
phone  = "${fake:phone(country=${addr.country_code})}"
person = "${fake:person(country=${addr.country_code})}"
```
