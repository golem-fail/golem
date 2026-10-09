### `browse_wait_not_exists` — Wait for an element to disappear

```toml
{ action = "browse_wait_not_exists", selector = ".spinner" }
```

The one thing no assertion does: `browse_assert_not_exists` answers "is it gone
now", this answers "let it finish going". Spinners, toasts and progress rows are
the reason it exists.

These wait on **DOM presence**, not visibility — an element hidden by CSS still
counts as present. Browser steps are instrumentation, and visibility judgements
belong to the mobile app under test.
