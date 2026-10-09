### `await_email` — Poll IMAP inbox

Poll an inbox over IMAP (TLS) and wait for an email matching the filters, with
optional regex extraction.

`inbox` is the **name of a variable** holding an inbox object, not the email
address. See [`create_inbox`](#create_inbox--provision-a-disposable-email-inbox)
for how the two pair up.

```toml
# Pairs with create_inbox { save_to = "inbox" }:
{ action = "await_email", inbox = "inbox", subject = "Verify*", timeout = 30000, save_to = "email" }

# …or a hand-written inbox object:
# [flow.vars]
# inbox = { imap_host = "imap.example.com", imap_port = "993", user = "me@example.com", pass = "secret" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `inbox` | — | Name of a variable holding an inbox object; the `imap_host` / `imap_port` / `user` / `pass` fields on it are used to connect |
| `recipient` | — | Glob filter for the recipient address (not `to`) |
| `subject` | `"*"` | Subject glob pattern |
| `extract` | — | Table of field names to regex patterns |
| `timeout` | `30000` | Polling timeout (ms) |

When more than one email matches, the **most recent** is returned, so a stale
match left in the inbox from an earlier run never shadows the fresh one. Only
the latest messages are scanned (not the entire mailbox), which is ample for
verification/OTP mail.
