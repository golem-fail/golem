### `create_inbox` — Provision a disposable email inbox

Provision a fresh inbox from a provider and save its connection details as an
object for later steps. The saved object's `imap_host`/`imap_port`/`user`/`pass`
fields are exactly what [`await_email`](#await_email--poll-imap-inbox) reads, so
`save_to = "inbox"` feeds straight into `await_email { inbox = "inbox" }`.

```toml
{ action = "create_inbox", provider = "ethereal", save_to = "inbox" }
{ action = "type", on_text = "Email", input = "${inbox.address}" }
# … app signup …
{ action = "await_email", inbox = "inbox", subject = "*verify*", extract = { otp = "code: (\\d{6})" }, save_to = "mail" }
{ action = "type", input = "${mail.otp}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `provider` | — | Inbox provider. Only `ethereal` is built in; any other value errors. |
| `save_to` | — | Variable to store the inbox object under (required). |
| `timeout` | `15000` | Provisioning deadline (ms). |

Saved object fields: `address` (= `user`, the email address), `user`, `pass`,
`imap_host`, `imap_port`, `smtp_host`, `smtp_port`.

> **Non-deterministic.** Provisioning is live network I/O, so the inbox is not
> replayed by `--seed` — each run gets a fresh address. Receiving mail at it is
> live too (`await_email` connects over real IMAP).
