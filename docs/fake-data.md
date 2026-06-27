# Fake Data Generators

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

## Contents

- [Determinism and seeds](#determinism-and-seeds)
- [All generators](#all-generators)
- [Simple generators](#simple-generators)
  - [email](#email)
  - [password](#password)
  - [uuid](#uuid)
  - [number](#number)
  - [one_of](#one_of)
  - [sentence](#sentence)
  - [phone](#phone)
- [Structured generators](#structured-generators)
  - [person](#person)
  - [address](#address)
  - [credit_card](#credit_card)
  - [timestamp](#timestamp)
- [Cross-references](#cross-references)

## Determinism and seeds

Generators draw from the run's random stream. Pass `--seed <N>` and every value
is **reproducible** — the same seed replays the same data, so assertions on
generated values stay stable. Without a seed, values are fresh each run, and the
seed actually used is reported so any run can be replayed.

**Time-based generators track "now" *and* reproduce.** `fake:timestamp` and a
card's expiry are anchored on a reference instant packed into the seed's high
bits (a 4-hour bucket since 2020). A no-`--seed` run anchors on the real current
time; replaying the reported seed reproduces the same dates bit-for-bit, because
the anchor rides inside the seed. (A hand-typed small seed like `--seed 42`
anchors at 2020 — consistent, just not "now".)

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

## Simple generators

One value each; all scalar except `timestamp`, which returns a small object
(its fields are below). Defaults are shown for each.

### email

`${fake:email}` → `<random>@example.com`. One string out, but the params cover
the common signup / inbox patterns:

| Param | Effect |
|-------|--------|
| `domain` | Replace `example.com` — `fake:email(domain=acme.test)` → `<random>@acme.test` |
| `prefix` | Prepend to the random local part — `fake:email(prefix=qa_)` → `qa_<random>@example.com` |

**Real-inbox / plus-addressing.** Put a `+` in `prefix` to keep every address
unique while delivering to a single real mailbox (Gmail-style aliasing):
`fake:email(prefix=alice+)` → `alice+<random>@example.com`, all routed to
`alice@…`. Combine with the [`await_email`](actions-reference.md) action to
verify mail end-to-end.

The random local part is 10–14 base-36 characters (~52–72 bits), comfortably
collision-free at golem's scale.

### password

`${fake:password}` → a random password. Params: `length` (default 12);
`symbols` (default `true` — set `symbols=false` for letters + digits only).
Example: `fake:password(length=20, symbols=false)`.

### uuid

`${fake:uuid}` → a v4 UUID, e.g. `f47ac10b-58cc-4372-a567-0e02b2c3d479`. No params.

### number

`${fake:number}` → a random integer (as a string). Params: `min` (default 0) and
`max` (default 100), inclusive. Example: `fake:number(min=1, max=6)`.

### one_of

`${fake:one_of(free|pro|enterprise)}` → one value picked at random from the set.
The natural fit for radio buttons, dropdowns, and enum fields — and a building
block for anything golem doesn't generate directly (a plan tier, a gender, a
subset of countries). Choices are `|`-delimited (commas also work, since `|`
sidesteps the param separator): `fake:one_of(yes|no)`, `fake:one_of(JP, US, GB)`.
Seeded like every other generator.

### sentence

`${fake:sentence}` → a short one-sentence filler string for textarea / bio /
comment / description fields. Default is **lorem ipsum**.

| Param | Effect |
|-------|--------|
| `language` | ISO 639-1 code — generate in that language. **110+ languages** supported (every code with a file in `data/sentences/` — en, es, zh, hi, ar, fr, pt, ru, ja, de, ko, sw, …). Omitted → lorem ipsum. An unsupported code is an error. |

`fake:sentence(language=ja)` → `古いゴーレムが石を砕く。`,
`fake:sentence(language=fr)` → `Le gardien garde la pierre.` Sentences are
golem-myth-themed (clay, stone, guardians) for amusement, script-correct
(CJK/Thai join without spaces, Arabic/Hebrew are right-to-left), and
seed-reproducible.

Grammar is deliberately simplified, and some less-common-script languages are
machine-authored pending native review — see the [roadmap](roadmap.md) if a
language's realism matters for your test.

### phone

`${fake:phone}` → a plausibly-formatted phone number.

| Param | Effect |
|-------|--------|
| `country` | ISO code; uses that country's dialing format — `fake:phone(country=JP)` → `+81-…`. Unset → a random country's format. An **unrecognised** code is an error (a typo shouldn't silently yield another country) |
| `format` | Explicit template where `#` becomes a random digit — `fake:phone(format=+1 (###) ###-####)`. Takes precedence over `country` |

Chain it off an address to keep them consistent:
`phone = "${fake:phone(country=${addr.country_code})}"`.

> **City / postcode / street** are not standalone generators. Use
> [`fake:address`](#address) dot-notation — `${fake:address.city}`,
> `${fake:address.postcode}`, `${fake:address.street}` — so all the parts of an
> address stay consistent (same city). For names, use
> [`fake:person`](#person).

## Structured generators

These return objects; read fields with `.field`.

### person

`${fake:person}` draws a given name and a family name from a single global pool
and makes each available in many writing systems. The pool is intentionally
diverse — it spans Latin (with the full range of diacritics), Japanese, Korean,
Chinese, Cyrillic, Arabic, Hebrew, Devanagari and Thai, and deliberately includes
the characters that break naive form validators (apostrophes like `O'Brien`,
hyphens like `Jean-Pierre`, the German `ß`).

A name's origin is **not** tied to the `country` parameter — people move. The
`country` describes the **form being filled in**: it decides which script the
form expects (so a foreigner's name romanises). It does **not** impose a full
name or an ordering — see [below](#full-name-build-it-yourself).

Three terms recur, in plain English: a **representation** is one way of writing a
part (its `native` form, an `ascii` fold, a `kana` reading…); a **chain** is an
ordered list of representations tried until one is non-empty; a **repertoire** is
the set of characters a country counts as native (its script(s) / accepted
accents). The common case needs none of this — the three fields below just work.

#### The fields

Every part comes as a `given` / `family` pair. The three you usually want:

| Field | What it is |
|-------|------------|
| `person.given` / `person.family` | **Primary** — what a local would type into the given/family field of a form in `country`. |
| `person.reading.given` / `.family` | **Reading / furigana**, where the form has one (Japanese katakana, Korean hangul). Empty otherwise. |
| `person.ascii.given` / `.family` | **Latin** — a pure-ASCII romanisation, always safe for an ASCII-only field. |

`person` is names only — for an email or phone use the dedicated `${fake:email}`
/ `${fake:phone(country=…)}` generators.

```toml
[flow.vars]
p = "${fake:person(country=JP)}"
# A Japanese name draws as kanji; a foreign name romanises on the JP form.
# ${p.given}          -> ゆき     (kanji)   OR  Jean   (foreigner, romanised)
# ${p.family}         -> 田中
# ${p.reading.given}  -> ユキ     (katakana furigana)
# ${p.ascii.family}   -> Tanaka
```

Every `person.X.Y` field is **always a string** — never undefined. When a part
doesn't apply (e.g. the reading on a form that has no reading field) it is the
empty string `""`.

#### Full name (build it yourself)

`person` deliberately exposes no joined "full name". A full name's *order* and
*separator* are properties of the **form**, not the name: Western forms write
`given family` with a space, but Japanese/Korean/Chinese run the parts together
(`田中ゆき`, `홍길동`) with no space. A test targets a known app, so build the
string the way that form wants it:

```toml
full_us = "${p.given} ${p.family}"   # given-first, space
full_jp = "${p.family}${p.given}"    # family-first, no space
```

#### How a part is resolved: representations and chains

Each field resolves through a **fallback chain** of representations: the first
that yields a non-empty value wins; if none do, the result is `""`.

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

Every representation is also exposed directly as `person.<rep>.{given,family}`,
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
approximation** from a stored IPA reading — not an authoritative spelling. So a
Korean person's `cyrillic` branch is a phonetic approximation; an empty `hanja`
just means that name has no stored Hanja.

#### Parameters

Three things are configurable, each overridable independently:

| Parameter | Sets | Example |
|-----------|------|---------|
| `country` | a **preset bundle** — the `name`/`reading` chains and the `local` repertoire at once | `country=JP` |
| `name` | the primary chain — an ordered fallback (overrides the country's) | `name=[local, ascii]` |
| `reading` | the reading chain | `reading=[katakana]` |
| `local` | the **character set** counted as native — a repertoire, *not* an ordered chain | `local=[kanji, hiragana]` |

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
# A katakana-only reading, regardless of the person:
furigana = "${fake:person(reading=[katakana]).reading.given}"

# Accept Latin names with French OR Portuguese accents, else romanise:
u = "${fake:person(local=[ascii, diacritics_fr, diacritics_pt]).given}"
```

#### Per-country behaviour

`country` presets live in the per-country `data/geo/*.json` files. The bracketed
lists below are literal token lists — the same syntax you'd pass to `name=` /
`reading=` / `local=`. A few:

| `country` | `local` repertoire | primary `name` | `reading` |
|-----------|--------------------|----------------|-----------|
| JP | `kanji`, `hiragana` | `[local, ascii]` | `[katakana]` |
| KR | `hangul` | `[local, ascii]` | `[hangul]` |
| CN | `hanzi` | `[local, ascii]` | — |
| RU | `cyrillic` | `[local, ascii]` | — |
| TH | `thai` | `[local, ascii]` | — |
| IN | `devanagari` | `[local, ascii]` | — |
| AE / EG | `arabic` | `[local, ascii]` | — |
| IL | `hebrew` | `[local, ascii]` | — |
| DE | `ascii`, `diacritics_de` | `[local, ascii]` | — |
| FR / CA | `ascii`, `diacritics_fr` | `[local, ascii]` | — |
| ES / MX | `ascii`, `diacritics_es` | `[local, ascii]` | — |
| BR | `ascii`, `diacritics_pt` | `[local, ascii]` | — |
| SE | `ascii`, `diacritics_sv` | `[local, ascii]` | — |
| IE / NZ / PL / LT / NL | `ascii`, `diacritics_<lang>` | `[local, ascii]` | — |
| BE | `ascii` + French/Dutch/German accents | `[local, ascii]` | — |
| US / GB / AU / ZA / SG | `ascii` | `[local, ascii]` | — |
| (none) | accepts everything | `[native]` | — |

### address

`${fake:address}` returns a coherent address for one place — all fields come from
the same city, so they stay consistent. Params: `country` (ISO code; unset → a
random country, an unrecognised code is an error), `state`, `region` (filter to a
state name / region tag). The `state` filter accepts either the native or the
romanised name (`東京` or `Tokyo`, case-insensitive).

The text fields default to the place's **native script**; an `ascii` sub-object
carries the romanised forms — exactly like `${fake:person}` (native default,
`.ascii` branch). Romanisations are ASCII folds of the native, never English
exonyms (`Bayern`, not "Bavaria"). Latin places (diacritics included) fold
programmatically; non-Latin places carry a stored romanisation.

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

### credit_card

`${fake:credit_card}` generates a Luhn-valid card. Params:

- `brand` — `visa` / `mastercard` / `amex` / `discover` (sets the number prefix
  and length).
- `provider` — `stripe` / `adyen` / `square` / … selects that payment provider's
  published **test-card** set, so `number` and `status` match what the provider's
  sandbox expects. Without it, a generic Luhn-valid card is produced.
- `status` — the simulated outcome (see below).

#### Fields

| Field | Example |
|-------|---------|
| `number` | 4532015112830366 |
| `expiry` | 03/28 |
| `cvv` | 123 |
| `brand` | visa |
| `status` | `""` (empty unless declined) |

#### Statuses

An approved card has an **empty** `status` (`""`). To simulate a failure, pass
`status=`; without a provider the options are `approved`,
`declined:invalid_number`, `declined:expired`, `declined:invalid_cvv`, `threeds`.
Provider-specific statuses vary by provider.

### timestamp

`${fake:timestamp}` returns a date/time object — read the field the form needs:

| Field | Example | Notes |
|-------|---------|-------|
| `.datetime` | `2025-09-14T08:21:00+00:00` | ISO 8601 |
| `.date` | `2025-09-14` | `YYYY-MM-DD` |
| `.time` | `08:21` | `HH:MM` |
| `.year` / `.month` / `.day` | `2025` / `09` / `14` | zero-padded parts — for forms with separate date inputs (e.g. date of birth) |

Address a field directly: `${fake:timestamp.date}` (the bare object errors in a
string context, like the other object generators).

Anchored on the run's reference instant (see
[Determinism and seeds](#determinism-and-seeds)) and **seed-reproducible**.
The date is drawn from a window measured **in whole years relative to the
anchor** — positive years are in the past, **negative years in the future**; by
default that is the last year.

| Param | Effect |
|-------|--------|
| `max_years` | The far edge (default 1). `fake:timestamp(max_years=5).date` → within the last 5 years |
| `min_years` | The near edge (default 0, i.e. the anchor) |

A **date of birth** is a window pushed into the past — set both edges:
`fake:timestamp(min_years=18, max_years=90).date` → someone aged 18–90 at the
anchor. A **future** date (a licence/subscription expiry up to 5 years out) uses
negatives: `fake:timestamp(min_years=-5, max_years=0).date`.

## Cross-references

Later generators can reference earlier variables:

```toml
[flow.vars]
addr   = "${fake:address(country=JP)}"
phone  = "${fake:phone(country=${addr.country_code})}"
person = "${fake:person(country=${addr.country_code})}"
```
