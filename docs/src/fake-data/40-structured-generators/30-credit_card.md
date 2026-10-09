### credit_card

`${fake:credit_card}` generates a Luhn-valid card. Params:

- `brand` — `visa` / `mastercard` / `amex` / `discover` (sets the number prefix
  and length).
- `provider` — `stripe` / `adyen` / `square` / … selects that payment provider's
  published **test-card** set, so `number` and `status` match what the provider's
  sandbox expects. Without it, a generic Luhn-valid card is produced. An unknown
  provider is an error, and the error message lists the valid providers (one per
  file in `data/cards/`, e.g. `pay_jp`, `checkout_com`).
- `status` — the simulated outcome (see below).

#### Fields

| Field | Example |
|-------|---------|
| `number` | 4532015112830366 |
| `expiry` | 03/28 |
| `cvv` | 123 |
| `brand` | visa |
| `provider` | `stripe`; `""` without a provider |
| `status` | `""` for an approved card without a provider; otherwise the status |
| `threeds` | `true` — present only when `status` is `threeds` (with a provider, any status starting with `threeds`) |

Some provider test cards carry extra fields the provider's form needs, such as
`name`, `postal_code`, `otp` or `pin`.

#### Statuses

Without a provider, an approved card has an **empty** `status` (`""`). To
simulate a failure, pass `status=`; without a provider the options are
`approved`, `declined:invalid_number`, `declined:expired`,
`declined:invalid_cvv`, `threeds`. Any other status needs a provider.

With a provider, `status` defaults to `approved`, and the valid statuses are the
keys of that provider's `statuses` table in `data/cards/<provider>.json` (Stripe,
for example, adds `declined:insufficient_funds`, `declined:lost`, `fraud`,
`threeds:failed`). A status the provider has no test card for is an error.
