### email

`${fake:email}` → `<random>@example.com`.

| Param | Effect |
|-------|--------|
| `domain` | Replace `example.com` — `fake:email(domain=acme.test)` → `<random>@acme.test` |
| `prefix` | Prepend to the random local part — `fake:email(prefix=qa_)` → `qa_<random>@example.com` |

**Real-inbox / plus-addressing.** Put a `+` in `prefix` and your mailbox's domain
in `domain` to keep every address unique while delivering to one real mailbox:
`fake:email(prefix=alice+, domain=gmail.com)` → `alice+<random>@gmail.com`, all
delivered to `alice@gmail.com`. Combine with the
[`await_email`](actions-reference.md#await_email--poll-imap-inbox) action to
verify mail end-to-end.

**Disposable real inbox.** `${fake:email}` has no inbox behind it; to receive
mail at a throwaway address, use the
[`create_inbox`](actions-reference.md#create_inbox--provision-a-disposable-email-inbox)
action.
