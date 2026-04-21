# Roadmap

## WebKit Inspector: Physical iOS Device Support

Currently WebKit Inspector enrichment (visible text, checked state) only works on iOS Simulator. The simulator exposes a Unix domain socket at `/private/tmp/com.apple.launchd.*/com.apple.webinspectord_sim.socket` which golem connects to directly.

Physical devices require a different transport path:

- **USB multiplexing** via `usbmuxd` — the system daemon that tunnels TCP over USB to iOS devices
- **Lockdown TLS handshake** — physical devices require a TLS connection using pairing certificates stored in `~/Library/Lockdown/`
- **Device discovery** — enumerate connected devices via usbmuxd, match to the target device

The `golem-driver/src/webkit.rs` transport layer is already designed around a `SimulatorTransport` trait, intended for a future `UsbTransport` implementation that handles the usbmuxd + TLS path.

Without this, physical device test runs still work but WebView elements lack enriched text — falling back to accessibility labels only.

Requires access to a physical iOS device for development and testing.

## CLI Flags: Not Yet Functional

Several CLI flags are defined but not yet wired through to execution.

### `--no-teardown` — Skip teardown blocks

Teardown blocks are parsed but never executed. The executor ignores the `teardown` field — no teardown logic runs after flows. The `no_teardown` config field is stored but there is nothing to skip.

### `--no-clean` — Skip app data clear

No app data cleaning logic exists in the execution path. The flag is accepted but there is nothing to skip.

### `--keep-devices` — Keep devices after completion

`auto_cleanup()` in golem-runner checks this flag, but `auto_cleanup()` is never called from the suite. Devices are released via resource manager but not shut down.

### `--record` — Auto screen recording

Flag is accepted but never triggers recording. Recording only works via explicit `start_recording`/`stop_recording` steps in flows.

### `--max-concurrency <N>` — Parallel device limit

Flag is defined but never read. `ResourceManager` uses default concurrency config regardless of this value.

## Flow Options: Not Yet Wired

These `[flow.options]` fields are parsed into `FlowOptions` but never read during execution.

### `record` / `recording_dir` — Auto recording

Both parsed but ignored. `CaptureConfig` hardcodes `record: false` and `recording_dir: .golem/recordings`. Recording only works via explicit `start_recording`/`stop_recording` steps.

`screenshot_dir` and `recording_dir` are superseded by the unified output directory design (see below).

## Ethereal Email Integration

`golem-email` crate has a working `EtherealClient` that creates temporary inboxes via the Nodemailer API (`https://api.nodemailer.com/user`), and an `ImapPoller` that polls IMAP for matching emails. Both are tested but not wired into the runner or generator system.

Intended usage: a `fake:email(ethereal=true)` parameter or a dedicated `fake:ethereal_email` generator that creates a real temporary inbox and exposes IMAP credentials as structured fields (`imap_host`, `imap_port`, `user`, `pass`). This would feed directly into `await_email`'s `inbox` parameter for end-to-end email verification flows.

This needs design work before implementation. The full email verification flow spans multiple concerns: creating the inbox, sending the email (via the app under test), polling for arrival, extracting content (verification URLs, OTP codes), and feeding extracted values back into the flow as variables. The `await_email` action already has `extract` (regex patterns) and `save_to`, but the end-to-end ergonomics — how a test author wires up `fake:email` → app signup → `await_email` → `open_link` — need to be planned as a cohesive feature.

Files: `golem-email/src/ethereal.rs`, `golem-email/src/imap_poller.rs`.

## iOS WebView: Slow Element Resolution Between Consecutive Actions

Consecutive `type` actions on iOS WebView elements are slow — resolving the second input field after typing in the first takes >10s. The DOM tree changes after each keystroke (WebKit enrichment re-fetches), and finding the next element requires waiting for the tree to settle.

Example: `e2e/cross/webview.test.toml` step 7 (`type on_text="Search"`) times out at 10s even though the previous `type` (step 5) completes in ~3.6s. The bottleneck is element resolution, not keystroke delivery.

Possible approaches:
- Smarter settle detection that recognizes when WebView content is still updating
- Cache element positions across consecutive steps when the viewport hasn't changed
- Longer default multiplier for WebView-context actions (requires detecting WebView context)

## Geo Data: Full-Width Number Support for Japanese Addresses

`expand_street_pattern()` generates ASCII digits (e.g. `清田一条2-7`), but real Japanese addresses often use full-width numbers (e.g. `清田一条２丁目７番`) or kanji numerals (e.g. `二丁目七番`). The current output looks unnatural for JP addresses.

Possible approaches:
- A `numeric_style` field per pattern or per country: `ascii` (default), `fullwidth`, `kanji`
- Post-processing step that converts ASCII digits to full-width (`0`→`０`, `1`→`１`, etc.) for JP patterns
- Update `jp.json` patterns to use full-width delimiters where appropriate (e.g. `丁目`, `番`, `号` instead of `-`)

May require updating both `expand_street_pattern()` and the JP geo data in `data/geo/jp.json`.

## Unified Output Directory

Replace separate `screenshot_dir` / `recording_dir` with a single `--output-dir` (default `.golem/results/`). Structure per-flow and per-device:

```
.golem/results/
  results.json
  results.xml
  login_test/
    iPhone_16e/
      screenshots/
        3_auth_block_0_1_error.png    # [3][auth_block:0][1] → global_block_iter_step_error
      recordings/
        recording.mp4
    Pixel_8/
      screenshots/
  checkout_test/
    ...
```

Screenshot filenames follow the `[global][block:iter][step]` output pattern: `3_auth_block_0_1_error.png`.

Device name (sanitized for filesystem) as subdir. Handles multiple devices of same platform. Reports at root — one per run. Each run overwrites same-named files; old orphans are harmless. A `golem clean` command or `--clean` flag can wipe the results dir.

Replaces the unwired `screenshot_dir` and `recording_dir` flow options.

## Orchestrator: Error Detail Not Forwarded to Client

When tests run via the orchestrator (multiple flows submitted to a running `golem.sock`), detailed error messages (step failures, timeout reasons, stack traces) go to the orchestrator's stderr — not the submitting client's output. The client only sees pass/fail summary lines.

The orchestrator protocol (`submit_and_wait`) streams pass/fail status but does not forward the per-device error context. This makes debugging failures in orchestrator mode harder than direct mode.

Fix: include error detail (failed block, step, reason) in the orchestrator's response stream alongside the pass/fail status.

