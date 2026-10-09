### `hide_keyboard` — Dismiss keyboard

Dismiss the on-screen keyboard. No-op if no keyboard is visible.

```toml
{ action = "hide_keyboard" }
```

> **Automatic keyboard recovery.** You rarely need an explicit
> `hide_keyboard` to reach a field the keyboard covers. During element
> resolution, if a target is absent from the keyboard-aware viewport but
> present in the unfiltered tree (i.e. the soft keyboard has occluded it),
> the resolver dismisses the keyboard **once per resolve** and re-polls. On
> Android the IME hide is animated and asynchronous, so the resolver waits
> for the reported keyboard height to return to 0 (with a timeout) before
> retrying, so the retap lands on the now-revealed field rather than the
> still-sliding panel.
>
> Set `keep_keyboard = true` on a step to opt out — of this recovery and of
> the pre-tap dismissal both. Use it when the step targets a keyboard
> accessory/toolbar control that acts on the focused field, or when the test
> is *about* keyboard-up state. An occluded target then stays unresolved
> rather than being reached by dismissing the keyboard:
>
> ```toml
> { action = "tap", on_text = "Done", keep_keyboard = true }
> ```
