# Roadmap

## `hardware` default — accept both, prefer virtual

`[[flow.apps.devices]] hardware` field today defaults to `[Some(false)]`
(virtual-only) when absent. That means a flow without an explicit
`hardware = "real"` line silently can't run on physical devices even
when one is connected. Better default: accept both kinds (virtual +
real) when absent, **prefer virtual** when both shapes are bootable
(so CI runs that have a sim and a phys connected pick the sim by
default for speed). Authors who need phys-only or virtual-only still
spell it out explicitly.

When this lands, the `push_notification` plan-time lint (added with
the action's sim/emu-only contract) needs its trigger condition
flipped: today it warns when `hardware` explicitly permits `real`;
post-change it should warn whenever `hardware` is absent **or**
explicitly permits real, since the absent case now also targets
phys.

**Files:** `golem-orchestrator/src/plan.rs::expand_hardware_entries`,
device-prefer logic in the resolver, `lint_push_notification_phys` (or
wherever the lint lives once added).

## Architecture and DX follow-ups from May 2026 review

Captured during the post-merge audit; none are blocking but each removes a sharp edge.

- **`is_debug` cross-crate coupling.** `golem-runner` reaches into `golem_driver::is_debug()` for a diagnostic eprintln. Move to `golem-common` or give the runner its own debug flag so the runner doesn't depend on the driver for telemetry.
- **`cssSafeAreaInset` invisible to callers.** Today the WebKit Inspector enrichment subtracts the inset locally and discards it. Adding `css_safe_area_top: i32` to `HierarchyMeta` (default 0) keeps the diagnostic record. Sets up Android once an equivalent surfaces.
- **`tap()` → `press(forDuration: 0.05)`.** Pages with a long-press distinguisher above ~50ms threshold may classify these as long-presses. Document the boundary or add an explicit `tap-fast` shorthand.
- **Resolver auto-hide-keyboard fires unconditionally.** Tests that intentionally exercise keyboard-up state will be perturbed. Consider an opt-out flag on the step or scope to specific actions.
- **`find_webview_socket` returns `None` on empty `pidof`.** Previously fell back to first-socket, useful for ad-hoc debugging. If we want to keep the loose path for `golem tree`, add a `--any` flag.
- **Tests gap.** `find_webview_socket` PID filter, safe-area subtraction, BUTTON/A textContent fallback, `EventLog`, `find_or_allocate_port` Android-only fallback, `ensure_companion_with_reg` UDID cross-check — none have unit coverage.
- **Docs gap.** `/press` companion endpoint, resolver auto-hide-keyboard — neither externally documented.

## Stale-bundle defense (Tauri iOS build pipeline)

`scripts/install-app.sh` and the corresponding template now (a) clear the per-arch build dir so the `tauri-cli` rename step succeeds, (b) prefer the per-arch path over the xcarchive copy when picking the produced `.app`, and (c) hard-fail when the picked `.app`'s mtime predates the build start. That closes the specific failure mode where weeks-old bundles were silently installed (see post-mortem: "menu missing" was actually "running an Apr 20 build for 3 weeks").

Further hardening that would catch the next variant of this class:

- **Content sanity hash.** Hash `test-app/dist/` after `npm run build` and verify the same hash appears as an embedded resource inside the `.app` (Tauri compresses the web bundle into the Rust binary, so we'd compute the hash on the source dist and embed it as a build-stamp the runner can `grep -F` for). Catches the case where Tauri produces a `.app` with empty/wrong web assets.
- **Reject `set +e` failures with a known signature.** The tolerated `tauri-cli` rename error is "failed to rename app ... Directory not empty". Instead of blanket-tolerating any nonzero exit, parse stderr and only tolerate that exact line. Anything else fails fast.
- **Build cache key includes lockfiles.** `install_cache.rs`'s fingerprint is git porcelain — works when lockfiles are tracked. When they're not (e.g. some downstream consumers), include lockfile hashes explicitly so `cargo update` / `npm install` invalidate the install cache.

## e2e Coverage for Physical Device Path

No e2e flow exercises the physical-device path today. Android is the easier starter (ADB-based, no special transport). Add:

- One flow under `e2e/physical/` that auto-skips when no physical device is connected (harness detects via `golem_devices::android::discover_physical_devices` — returns empty → mark as xfail).
- CI runs it only on a self-hosted runner with a real device attached.
- Verifies the physical path works for basic `tap`/`type` — no WebView yet (blocked by iOS WebKit work).

**Files:** `e2e/physical/basic.test.toml` (new), `.github/workflows/` (physical-runner lane, gated).

## WebKit Inspector: Physical iOS Device Support

Currently WebKit Inspector enrichment (visible text, checked state) only works on iOS Simulator. The simulator exposes a Unix domain socket at `/private/tmp/com.apple.launchd.*/com.apple.webinspectord_sim.socket` which golem connects to directly.

Physical devices require a different transport path:

- **USB multiplexing** via `usbmuxd` — the system daemon that tunnels TCP over USB to iOS devices
- **Lockdown TLS handshake** — physical devices require a TLS connection using pairing certificates stored in `~/Library/Lockdown/`
- **Device discovery** — enumerate connected devices via usbmuxd, match to the target device

The `golem-driver/src/webkit.rs` transport layer is already designed around a `SimulatorTransport` trait, intended for a future `UsbTransport` implementation that handles the usbmuxd + TLS path.

Without this, physical device test runs still work but WebView elements lack enriched text — falling back to accessibility labels only.

Requires access to a physical iOS device for development and testing.

## True Parallel Flow × Device Concurrency

Running `golem run a.toml b.toml` on ios+android = 4 device-runs available but only 2 execute in parallel (one per booted device per platform). Other 2 wait for devices to free. Machines with spare RAM could run all 4 at once.

**Desired:** Boot additional simulators/emulators on demand when:
- `total_device_runs > currently_booted_matching_devices` AND
- Free RAM above threshold (per-device ~2-4GB)

**Limits:** `--max-concurrency <N>` always caps — if N is lower than the heuristic allows, N wins. Default stays 4.

**Cleanup:** Track which sims/emulators golem booted (vs user's) so they can be shut down afterwards. Respects `--keep-devices`.

**Note:** Works with the existing install script support — fresh sims booted on-demand will have their install_script invoked automatically via the existing pipeline.

**Files:** `golem-devices/src/resource_manager.rs` (boot-on-demand logic), `golem-devices/src/concurrency.rs` (headroom checks), `golem-devices/src/{ios,android}.rs` (boot helpers + tracking).

## Coverage Strategy: Residual Polish

The tick-box model is live end-to-end: `CoverageStrategy { One, Min, Smart, Full }` with `Smart` default, `DeviceSlot` with `Option<Platform>` + `booted` axis, greedy set-cover in `golem-orchestrator::coverage`, partial-axis expansion with dedup + underspec errors, and execute-time adaptive JIT for `Smart` + `One` via `CoverageGroup` + shared progress tracker in the scheduler. `FlowReport.covered_axes` is populated from the chosen device and renders in human output.

**What's left:**

### Responsive-design / cross-platform axis sharing

Already works via array syntax on a single `[[flow.apps.devices]]` block: `os = ["ios:latest", "android:latest"]` + `type = ["phone", "tablet"]` emits 4 partial boxes that 2 devices (one per platform, different types) cover end-to-end. Documented in README flow-options.

### Reference: strategy semantics

| Strategy | Box generation | Resolution timing | Semantics |
|---|---|---|---|
| `full` | Cartesian — each box fully pinned | plan-time | 1 FlowRun per box |
| `min` | Partial-axis — each axis-value = one box | plan-time: greedy set-cover | Fewest devices; waits on contested |
| `smart` | Partial-axis | execute-time adaptive (CoverageGroup) | **Default.** Stops once every pool box is ticked |
| `one` | Partial-axis | execute-time adaptive (CoverageGroup, `max_runs=1`) | Single successful run; local smoke / dev |

## Partial Suite on Install Failure

If pre-install fails for app `X` on device `D`, today's per-flow install check marks `FailedScript` in the cache and any flow referencing `X` on `D` is skipped. But UX could be sharper:

- Dedicated `FlowSkipped` event with explicit cause (`InstallFailed(X, D)` vs other skip reasons)
- Aggregated suite summary line distinguishing "install-dep-skip" from genuine flow failures
- Flows that don't reference `X` proceed normally (already the case)

**Foundation:** `InstallCache` already keyed on `(udid, bundle)`; per-flow skip logic already exists. This is polish.

**Files:** `golem-events/src/lib.rs` (enrich `FlowSkipped` variant), `golem-report/src/stream.rs` + `accumulator.rs` (render distinct skip reasons).

## Persistent Install Cache: Polish

The persistent install cache is shipped (`.golem/install-cache.json`, three integrity gates, `--rebuild`, `--no-build`). Remaining polish:

- **Surface skipped installs in JSON / JUnit / TOON reports.** Today the live stream prints `[install] ... — skipped (cache hit ...)` but the persistent reports list cached installs as silent — they don't appear in `installs[]`. Add `skipped: bool` + `skip_reason: Option<String>` to `InstallReport` and wire it through the four serialisers. Useful for CI artifacts where reviewers want to confirm nothing was rebuilt.
- **`golem cache clear` subcommand** — only if shared-CI long-running orchestrator surfaces a real workflow. Today `rm .golem/install-cache.json` is enough.
- **Cache size diagnostics** — `golem cache info` (or under `--verbose`) printing entry count + last-updated dates. Low-priority debugging aid.

## Migrate SuiteRunner + IPC into `golem-orchestrator`

`SuiteRunner` lives in `golem-cli/src/suite.rs` and IPC logic in `golem-cli/src/orchestrator.rs`. Both belong in the orchestrator crate — cleanly separates glue (CLI arg parsing, output rendering) from core (suite execution, multi-process coordination).

**Files:** move `golem-cli/src/suite.rs` → `golem-orchestrator/src/suite.rs`; move `golem-cli/src/orchestrator.rs` → `golem-orchestrator/src/ipc.rs`.

## Force Separate Device per App (`share_device = false`)

By default, `[[flow.apps]]` entries whose device constraints are jointly satisfiable pack into the same physical device to save host resources. For some flows this default is wrong — for example a deep-link test where two apps must be on different devices to exercise cross-device IPC, or an isolation test where sharing a device would contaminate state.

**Proposed TOML:**
```toml
[[flow.apps]]
name = "a"
share_device = false     # opt out of packing — a gets its own device
```

**Implementation:** add `share_device: Option<bool>` (default = true) to `AppConfig` in golem-parser. The `golem-orchestrator` Plan generator honours it when building slot groupings — an app with `share_device = false` always gets its own `DeviceSlot`.

**Where it lands:** the Plan generator already groups `[[flow.apps]]` into `DeviceSlot`s; `share_device = false` becomes an input to that grouping pass.

## Multi-Device Flow Coordination (Chat Tests)

Some flows use two apps on two different devices that must run together (chat client + chat server). Today's suite model spawns a separate flow task per platform; two devices never coordinate inside one flow execution. The new `FlowRun { slots: Vec<DeviceSlot> }` structure supports 2+ slots, but the initial Plan implementation only emits single-slot FlowRuns.

**Implementation:**
- Plan generator detects apps with incompatible `[[flow.apps.devices]]` constraints (e.g. different platforms) and emits one `FlowRun` with a `DeviceSlot` per incompatible group.
- Execute phase acquires ALL slots' devices before starting the flow; runs the flow with multi-device context (flow steps can `{ action = "launch", app = "b" }` to switch focus between devices).
- Device release happens after the whole FlowRun completes, not per-slot.

**Depends on:** clarification of flow-step semantics across devices — which device is "current" at each step, how `{ action = "launch", app = "b" }` switches focus, how assertions scope. The slot infrastructure already exists; the missing piece is step-level semantics.

## Reconcile `[[flow.apps]]` Implementation with Original Spec

Per design notes, some original-spec behaviour around `[[flow.apps]]` and step-level app targeting was not carried through during initial implementation. Examples:
- Default expectation that blocks/steps target the single app when only one is declared.
- `{ action = "launch", app = "app-b" }` switching apps on the same device.
- Device-sharing defaults for multi-app flows.

**Action:** produce a reconciliation doc mapping current implementation against original spec, flag gaps, and either (a) fix the implementation, or (b) update the spec to match current behaviour with a rationale. Low priority but important for long-term clarity.

**Files:** `docs/reconciliation-flow-apps.md` (new), followed by targeted fixes in `golem-parser/src/lib.rs` or `golem-runner/src/executor.rs` depending on findings.

## CLI Flags: Not Yet Functional

Several CLI flags are defined but not yet wired through to execution.

### `--no-teardown` — Skip teardown blocks

Teardown blocks are parsed but never executed. The executor ignores the `teardown` field — no teardown logic runs after flows. The `no_teardown` config field is stored but there is nothing to skip.

### `--no-clean` — Skip app data clear

No app data cleaning logic exists in the execution path. The flag is accepted but there is nothing to skip.

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

## TOON Timestamp Representation

Human, JSON, and JUnit outputs already emit wall-clock timestamps (local `HH:MM:SS.mmm` prefix for human; ISO-8601 UTC on every report level for JSON/JUnit). TOON is intentionally left out — its compactness-first design would bloat meaningfully with full ISO-8601 strings (16+ chars) per entry.

**Open proposal:** emit a single suite-level `start` unix-epoch timestamp once, then per-event `delta_ms` relative to suite start. A 30-minute suite = 7-digit delta; easy to reconstruct absolute time from `start + delta_ms`.

Needs a concrete schema decision before implementing. Today TOON emits `duration_ms` only.

**Files:** `golem-report/src/toon.rs` once schema is agreed.

## Skipped Step Reasons Across All Outputs

Skipped steps carry no reason today: a ` -tap:Cancel` line in TOON (or `<skipped/>` in JUnit, `"outcome": "skipped"` in JSON) tells the reader *that* a step was skipped, not *why*.

**Fix:** add `skip_reason: Option<String>` to `StepReport`. Populate from flow execution when a step is conditionally skipped (e.g. `if:` predicate false, barrier-aborted, start-block cursor past this step). Surface per renderer:

- Human stream: `─ tap:Cancel (skipped: barrier aborted)`
- JSON: `"skip_reason": "barrier aborted"`
- JUnit: `<skipped message="barrier aborted"/>`
- TOON: ` -tap:Cancel :barrier_aborted` (short reason token; long reasons truncated)

Also roadmap-adjacent: consider whether `golem_events::StepOutcome::Skipped` should become `Skipped(String)` symmetrically with `Warning(String)` / `Failed(String)`, or keep `Skipped` as-is and pass the reason via a sibling event/field. Second option keeps the common case small.

**Files:** `golem-events/src/lib.rs` (StepOutcome shape), `golem-runner/src/executor.rs` (populate reasons at skip decision), `golem-report/src/{accumulator,human,json,junit,toon}.rs` (surface).

## Transient Install Errors: Retry Classifier Polish

`golem-cli/src/suite.rs::is_transient_install_error` classifies a small set of known-recoverable install-script error patterns and retries the script once with `install_only=true` (reusing the already-built artifact). Currently matches:

- `Mach error -308 (ipc/mig) server died` / `NSMachErrorDomain code=-308` — CoreSimulator IPC blip on freshly-booted iOS sims
- `error: device offline` / `error: device not found` — adb device-state race during emulator early boot

**What's left:**
- Add an iOS-side grace probe after `bootstatus -b` (e.g. `xcrun simctl getenv <udid> HOME` until fast) to potentially eliminate the Mach -308 case at source rather than retrying after.
- Expand the classifier as new transient patterns surface in CI logs. Conservative — adding patterns that aren't actually recoverable just masks real errors behind a 3s delay.

## iOS 26 + WebView: Auto-Scroll Past Inner Scrollables Fails

iOS 26 simulator + Tauri WebView: `auto_scroll = true` repeatedly fails to scroll past an inner scrollable into the lower part of the page. `dialog_overlay`, `read`, `scroll_search`, `wait` (`Show Delayed`), `webview` all hit this. Android passes the same flows cleanly.

The scroll loop logs `inner scrollable consumed gesture` and switches strategies, but never reaches the target. Same root family as the existing "iOS WebView slow element resolution" entry — both are WebKit Inspector + scroll/settle interactions on iOS 26.

**Files:** `golem-runner/src/scroll.rs` strategy switching; `golem-driver/src/webkit.rs` for inspector tree freshness during scroll.

## iOS concurrent flows: cross-flow focus / state corruption

When iPhone + iPad run flows in parallel, occasional state leaks between sims:

- **Wrong-field type:** observed once on iPhone 17 — typing for `Password` landed in the `Search` input. The next field's focus snapshot apparently lagged by one step, so `typeText` delivered keystrokes to the previously-focused field instead.
- **Step-6 backspace flake:** one of the two flows occasionally times out at `backspace on_text="golem testt"` — element resolves but the action stalls past the step deadline. Solo runs never trigger.
- **Step-19 auto_scroll for Submit:** scroll loop enters strategy 2 stalls under concurrent load even after our scroll-strategy fix.

The companion-side off-main fix (commit on this entry's removal) prevents one wedge from cascading into all later requests, but doesn't address the underlying issue: XCUITest's HID injection and accessibility-snapshot paths are process-global. When two sims drive XCUITest concurrently from the same host, they interleave on shared `simctl` / `usbmuxd` / `IOHIDEvent` plumbing. Apple's official guidance is one XCUITest run per host process — we're stretching that.

Likely shape of the real fix: serialise the host-side simctl-touching operations (mainly tap synthesis + window-snapshot probes) behind a host-wide mutex, or run each sim's companion in a separate XCUITest process so OS-level state is per-process.

Not blocking — single-device runs are stable, multi-device retry-flaky.

**Files:** `companions/ios/GolemRunnerUITests/RequestRouter.swift`, `golem-driver/src/ios.rs` (a host-wide mutex would live here).

## Test App: Menu nav migration — remaining flows

Menu nav (`tap on_accessibility_label="menu-toggle"` + `tap on_accessibility_label="goto-X"`) replaces `auto_scroll = true` for non-scroll-testing flows.

**Still on auto_scroll:** `device_controls` (auto_scroll struggles to reach `Theme:` row on iPhone — strategy 1 stalls, strategy 2 stalls; separate scroll-strategy bug, not menu nav). Intentionally on auto_scroll: `scroll.test`, `scroll_search.test`.

## Android: AndroidManifest permission persistence

`pm grant` requires `<uses-permission>` declarations in `AndroidManifest.xml`. The test-app currently has CAMERA / RECORD_AUDIO / ACCESS_FINE_LOCATION / ACCESS_COARSE_LOCATION declared in `test-app/src-tauri/gen/android/app/src/main/AndroidManifest.xml`, but `gen/` is gitignored — fresh clones lose the declarations and `permissions_*.test` will fail at the grant step.

Tauri 2.x has no first-class config for Android `<uses-permission>`. Options:

- Commit `test-app/src-tauri/gen/android/` (standard for many Tauri 2.x projects)
- Add a `build.rs` / pre-build script that patches the manifest
- Wait for upstream Tauri to expose `tauri.conf.json` → `bundle.android.permissions`

**Files:** `.gitignore`, `test-app/src-tauri/gen/android/app/src/main/AndroidManifest.xml`.

## Android: sticky menu tap target only half-clickable

On Android phone with the sticky `Menu.svelte` at `scrollTop = 0`, roughly the top half of the menu-toggle button overlaps the system status bar / notification area and is not tappable by a human (the OS intercepts touches). Tests work because the companion's `tap` syntheses go through to the WebView regardless. Pure UX issue for manual testing.

The `padding: max(8px, env(safe-area-inset-top, 8px))` already shifts the button down some — but Android's `env(safe-area-inset-top)` reports 0 on most emulators, so the padding doesn't compensate.

**Files:** `test-app/src/lib/Menu.svelte`.

