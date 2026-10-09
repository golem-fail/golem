### sentence

`${fake:sentence}` → a short one-sentence filler string for textarea / bio /
comment / description fields. Default is **lorem ipsum**.

| Param | Effect |
|-------|--------|
| `language` | ISO 639-1 code — generate in that language. **110+ languages** supported (every code in the bundled sentence corpus — en, es, zh, hi, ar, fr, pt, ru, ja, de, ko, sw, …). Omitted → lorem ipsum. An unsupported code is an error. |

`fake:sentence(language=ja)` → `古いゴーレムが石を砕く。`,
`fake:sentence(language=fr)` → `Le gardien garde la pierre.` Sentences are
golem-myth-themed (clay, stone, guardians), script-correct
(CJK/Thai join without spaces, Arabic/Hebrew are right-to-left), and
seed-reproducible.

Grammar is deliberately simplified, and some less-common-script languages are
machine-authored pending native review.
