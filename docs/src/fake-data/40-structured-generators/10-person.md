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

`country` presets live in the bundled locale data. The bracketed
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
