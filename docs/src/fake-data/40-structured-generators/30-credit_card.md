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
