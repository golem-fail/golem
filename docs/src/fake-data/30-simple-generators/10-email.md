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
`alice@…`. Combine with the [`await_email`](actions-reference.md#await_email--poll-imap-inbox)
action to verify mail end-to-end.

**Disposable real inbox.** `${fake:email}` is a synthetic string — no inbox
exists behind it. To provision a *live* throwaway mailbox you can actually
receive at (for OTP / verification-link flows), use the
[`create_inbox`](actions-reference.md#create_inbox--provision-a-disposable-email-inbox)
action instead — it's a network side effect, not a seed-reproducible generator,
so it lives as an action rather than a `${fake:…}` value.

The random local part is 10–14 base-36 characters (~52–72 bits), comfortably
collision-free at golem's scale.
