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

## Device Resolution: `ios:latest` Prefers Booted Over Newest Version

`resolver.rs` sorts candidates by state preference (booted > shutdown) **before** filtering to `:latest`. Result: if iOS 18 simulator is booted and iOS 26 simulator is shutdown, `os = "ios:latest"` picks iOS 18. The `:latest` directive is ignored in favor of "already running".

**Desired:** `:latest` means highest version. Match version requirement first, then tie-break by state only within the same version.

**Files:** `golem-devices/src/resolver.rs` (min-coverage loop tie-break logic, lines ~281-410), `golem-devices/src/version.rs` (latest resolution).

## Multi-Flow Output Broken in Direct Suite Mode

Running `golem run a.toml b.toml` (2+ flows without a running orchestrator) shows device discovery messages twice ("Platform: ios", "Companion: ios"), then floods "Waiting for resources (ios)..." repeatedly. No flow/step events render. Tests **do** run — just no streaming output to stderr. Single-flow runs work fine.

**Cause:** `run_suite()` spawns each flow as a tokio task calling `run_single_flow_with_resources()`. Each flow creates its own `event_channel` and its own `stream_human` subscriber (`golem-cli/src/suite.rs:339-354`). Two concurrent human renderers write to stderr and collide. The `multi_device` prefix logic handles one flow's multiple devices but doesn't know about flow-level prefixing.

**Fix:** One event channel per suite (not per flow). Single `stream_human` subscriber for all flows. Prefix events with flow name when `flow_count > 1` (same pattern as `multi_device` prefix). Mirror the orchestrator client-side streaming which already uses a single local renderer.

**Files:** `golem-cli/src/suite.rs` (run_suite, run_single_flow_with_resources), `golem-report/src/stream.rs` (flow prefix logic).

## FailureBarrier Across Multi-Flow Concurrency

`FailureBarrier` (`golem-runner/src/barrier.rs`) coordinates devices within a single flow — device A fails at step 7, devices B/C abort at step ≥7.

With multi-flow parallelism the scope needs to stay per-flow: failure on `flow_a/ios` should abort `flow_a/android` but NOT `flow_b/ios`. Current barrier is per-flow, cloned to device tasks — should still work but needs doc clarity and a regression test.

**Action:** Keep barrier per-flow. Document semantics. Add test covering 2 flows × 2 devices where one flow fails; verify other flow completes unaffected.

**Files:** `golem-runner/src/barrier.rs` (doc comments), new test under `golem-runner/tests/`.

## App Install via User-Provided Bash Script

Currently the app under test is assumed pre-installed. If golem boots a fresh simulator/emulator, `launch` will fail. Conventional install paths won't work across frameworks (Expo Go, Expo Dev Client, React Native bare, native Swift/Kotlin, Flutter, multi-app repos, dev vs release builds).

**Design direction (needs discussion before implementation):**
- Dev writes a bash script (committed to repo) that golem invokes per-device
- Golem passes `device_id` (and platform) as arguments
- Script does framework-specific build + install, echoes installed bundle path/id on success
- `[flow.apps]` gains `install_script = "scripts/install.sh"` (or similar)
- `golem create`-style CLI command to scaffold starter script per framework

**Open questions:**
- Incremental build caching — script's problem, or golem-level hash check?
- Script interface: args vs env vars vs stdin protocol
- Install vs launch separation — script installs only, golem launches?
- Error reporting from script — exit codes + stderr pass-through?
- Multi-app flows — one script per app or one unified script?

**Files (likely):** `golem-cli/src/suite.rs` (flow startup invokes script), `golem-driver/src/{ios,android}.rs` (install from script-provided path), `golem-parser/src/lib.rs` (AppConfig.install_script field), `golem-cli/src/scaffold.rs` (script scaffolding).

## True Parallel Flow × Device Concurrency

Running `golem run a.toml b.toml` on ios+android = 4 device-runs available but only 2 execute in parallel (one per booted device per platform). Other 2 wait for devices to free. Machines with spare RAM could run all 4 at once.

**Desired:** Boot additional simulators/emulators on demand when:
- `total_device_runs > currently_booted_matching_devices` AND
- Free RAM above threshold (per-device ~2-4GB)

**Limits:** `--max-concurrency <N>` always caps — if N is lower than the heuristic allows, N wins. Default stays 4.

**Cleanup:** Track which sims/emulators golem booted (vs user's) so they can be shut down afterwards. Respects `--keep-devices`.

**Depends on:** App install script support (previous entry) — without it, fresh sims have no app.

**Files:** `golem-devices/src/resource_manager.rs` (boot-on-demand logic), `golem-devices/src/concurrency.rs` (headroom checks), `golem-devices/src/{ios,android}.rs` (boot helpers + tracking).

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


