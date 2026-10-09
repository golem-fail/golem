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
