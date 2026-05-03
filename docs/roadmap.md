# Roadmap

## iOS companion HTTP server runs on main thread

`RequestRouter.swift` dispatches every handler with `DispatchQueue.main.sync { ... }`. Once one handler wedges (e.g. `XCUIApplication.typeText` blocking on a soft-keyboard race), the whole HTTP loop wedges with it — every subsequent request from the runner stalls until reqwest's per-request timeout fires (now wired up via `CompanionClient::set_request_timeout`, but each step still burns its full deadline).

Real fix: serve the HTTP loop off main, hop to main only for XCUITest calls that actually need it, with a per-handler watchdog so a stuck `typeText` returns 504 instead of permanently freezing the harness.

**Files:** `companions/ios/GolemRunnerUITests/RequestRouter.swift`, `companions/ios/GolemRunnerUITests/RequestHandlers/*.swift`.

## Architecture and DX follow-ups from May 2026 review

Captured during the post-merge audit; none are blocking but each removes a sharp edge.

- **`is_debug` cross-crate coupling.** `golem-runner` reaches into `golem_driver::is_debug()` for a diagnostic eprintln. Move to `golem-common` or give the runner its own debug flag so the runner doesn't depend on the driver for telemetry.
- **`cssSafeAreaInset` invisible to callers.** Today the WebKit Inspector enrichment subtracts the inset locally and discards it. Adding `css_safe_area_top: i32` to `HierarchyMeta` (default 0) keeps the diagnostic record. Sets up Android once an equivalent surfaces.
- **iOS `handleLaunch` worst-case 15s wait.** Three `waitForExistence(timeout: 5.0)` probes in series. Apps with no static text on first paint pay the full third probe before `/launch` returns. Tighten the `staticTexts` timeout or make it configurable per-app.
- **Companion tap waits up to 2s for `windows.firstMatch`.** `handleTap` / `handleLongPress` / `handleHideKeyboard` each block for up to 2s when the window query lags. Long flows pay this per action.
- **`tap()` → `press(forDuration: 0.05)`.** Pages with a long-press distinguisher above ~50ms threshold may classify these as long-presses. Document the boundary or add an explicit `tap-fast` shorthand.
- **Resolver auto-hide-keyboard fires unconditionally.** Tests that intentionally exercise keyboard-up state will be perturbed. Consider an opt-out flag on the step or scope to specific actions.
- **`find_webview_socket` returns `None` on empty `pidof`.** Previously fell back to first-socket, useful for ad-hoc debugging. If we want to keep the loose path for `golem tree`, add a `--any` flag.
- **`normalize_android_permission` typo passthrough.** A misspelled `"locaiton"` is forwarded to `pm grant` verbatim. Either error on unknown shorthands or document the passthrough explicitly.
- **`location-always` collapses to `ACCESS_FINE_LOCATION`.** Always-on location actually needs `ACCESS_BACKGROUND_LOCATION` too. Collapse is silent.
- **`photos` shorthand picks `READ_MEDIA_IMAGES` only.** Android 12 and below want `READ_EXTERNAL_STORAGE`; Android 14+ has `READ_MEDIA_VISUAL_USER_SELECTED`. API-level-blind today.
- **No iOS analogue to `normalize_android_permission`.** iOS path passes the shorthand verbatim to `simctl privacy`; works for `camera` / `location` but silently diverges from Android for less common ones.
- **Tests gap.** `normalize_android_permission`, `find_webview_socket` PID filter, safe-area subtraction, BUTTON/A textContent fallback, `EventLog`, `find_or_allocate_port` Android-only fallback, `ensure_companion_with_reg` UDID cross-check — none have unit coverage.
- **Docs gap.** `/press` companion endpoint, resolver auto-hide-keyboard, the now-required `app =` field on `grant_permission` / `revoke_permission` actions — none are externally documented.
- **`tauri-plugin-deep-link::register` never invoked.** Flagged during deep-link investigation. iOS plumbing is broken regardless (see Deep-link entry below); decide and document whether `register()` would help on Android.
- **`Menu.svelte` `scroll-margin-top` hard-codes 60px.** Refactors that grow the menu height regress scroll-into-view. Compute from the menu's bounding box.
- **`EventLog.MAX = 50`.** Pointermove bursts evict prior events. Acceptable for a debug tool today; bumping to time-windowed (last 5s) would survive long flows.

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

## iOS WebView: Slow Element Resolution Between Consecutive Actions

Consecutive `type` actions on iOS WebView elements are slow — resolving the second input field after typing in the first takes >10s. The DOM tree changes after each keystroke (WebKit enrichment re-fetches), and finding the next element requires waiting for the tree to settle.

Example: `e2e/cross/webview.test.toml` step 7 (`type on_text="Search"`) times out at 10s even though the previous `type` (step 5) completes in ~3.6s. The bottleneck is element resolution, not keystroke delivery.

Possible approaches:
- Smarter settle detection that recognizes when WebView content is still updating
- Cache element positions across consecutive steps when the viewport hasn't changed
- Longer default multiplier for WebView-context actions (requires detecting WebView context)

The native `await_first_frame` settle gate (in `golem-driver/src/lib.rs`) handles the launch → first-action race for native screens. Extending the same pattern to also poll WebKit Inspector readiness when a WebView is present would close the WebView gap.

## Transient Install Errors: Retry Classifier Polish

`golem-cli/src/suite.rs::is_transient_install_error` classifies a small set of known-recoverable install-script error patterns and retries the script once with `install_only=true` (reusing the already-built artifact). Currently matches:

- `Mach error -308 (ipc/mig) server died` / `NSMachErrorDomain code=-308` — CoreSimulator IPC blip on freshly-booted iOS sims
- `error: device offline` / `error: device not found` — adb device-state race during emulator early boot

**What's left:**
- Add an iOS-side grace probe after `bootstatus -b` (e.g. `xcrun simctl getenv <udid> HOME` until fast) to potentially eliminate the Mach -308 case at source rather than retrying after.
- Expand the classifier as new transient patterns surface in CI logs. Conservative — adding patterns that aren't actually recoverable just masks real errors behind a 3s delay.

## iOS 26 Tap on `+` Doesn't Register After UI Fully Rendered

`e2e/perf/tap_roundtrip.test.toml` and several other flows on iOS 26: launch → wait Counter (✓) → assert "0" (✓) → next `tap on_text="+"` times out 5s. The element resolves visually (other steps in the same screen state work), but tap to the same coords doesn't register an increment. Pre-existing on iOS 26; iOS 18 in the same flows works.

Affected flows: `tap_roundtrip`, `mixin`, `multi_app_switching`, `alerts`, others involving `+`.

**Settle gate (`await_first_frame`) is in place** — the tree is fully settled before the failing tap fires. The cause is iOS 26 specific: the companion's tap path may use an API that behaves differently on iOS 26, or `+` has an accessibility-tree representation that the tap-to-element resolution can't reach. Investigate by adding a tree-dump immediately before the failing tap to capture the state.

**Files:** `golem-driver/src/ios.rs` tap path.

## iOS 26 + WebView: Auto-Scroll Past Inner Scrollables Fails

iOS 26 simulator + Tauri WebView: `auto_scroll = true` repeatedly fails to scroll past an inner scrollable into the lower part of the page. `dialog_overlay`, `read`, `scroll_search`, `wait` (`Show Delayed`), `webview` all hit this. Android passes the same flows cleanly.

The scroll loop logs `inner scrollable consumed gesture` and switches strategies, but never reaches the target. Same root family as the existing "iOS WebView slow element resolution" entry — both are WebKit Inspector + scroll/settle interactions on iOS 26.

**Files:** `golem-runner/src/scroll.rs` strategy switching; `golem-driver/src/webkit.rs` for inspector tree freshness during scroll.

## iPad WKWebView: menu-nav after auto_scroll-into-counter

Now that iPad routing is fixed (above), `webview.test` on iPad gets through 15 of its 27 steps and fails at the first `tap on_accessibility_label="menu-toggle"` after `tap Increment` with `auto_scroll = true`. Suspect: iPad's larger viewport + scroll-into-Counter leaves the sticky menu in a different bounds box than the resolver expects, OR iPad's a11y tree exposes the menu differently.

Repro:
```
golem run e2e/cross/webview.test.toml --platform ios --coverage one
```
fails at step 15 `tap menu-toggle` (label `[ios/iPad (A16)]`).

**Files:** `test-app/src/lib/Menu.svelte` (sticky-menu CSS), `golem-driver/src/webkit.rs` (iPad inspector enrichment).

## Deep-link delivery on iOS — two stacked blockers

Investigated end-to-end. The JS listener wiring is fine; two separate iOS-level problems sit between `simctl openurl` and `onOpenUrl` in JS.

**Blocker 1: `simctl openurl` triggers an iOS confirmation dialog.** For a custom URL scheme delivered from outside the app (simctl counts as outside), iOS shows `Open in "GOLEM Test App"?` with Cancel/Open. Until "Open" is tapped, the URL never reaches the app process. golem's `open_url` action (driver-side) just calls `simctl openurl` and returns — doesn't dismiss the dialog.

**Blocker 2: even after tapping "Open", the URL doesn't reach the JS listener.** With the test-app rebuilt to expose plugin-side state in the DOM:
- `import('@tauri-apps/plugin-deep-link')` succeeds (keys: `getCurrent, isRegistered, onOpenUrl, register, unregister`).
- `onOpenUrl(handler)` returns successfully (`listener-ok`).
- `getCurrent()` returns `null` on cold-start (after dialog confirm-Open) AND on warm-start.
- `onOpenUrl` callback never fires after openurl + dialog confirm.
- `isRegistered("golem-test")` is permission-blocked (`deep-link:allow-is-registered` not in `capabilities/default.json`'s `deep-link:default` set), but that's an unrelated minor gap.

The Tauri plugin's iOS path (Tauri 2.x → Tao → UIApplicationDelegate hook → plugin event → JS) appears to silently drop the URL. This is in `tauri-plugin-deep-link 2.4.7` territory; not a test-app config we can tweak from outside.

**Files / next attempts (when picked back up):**
- `golem-driver/src/ios.rs::open_url` — auto-tap "Open" on the iOS confirmation dialog (simulator-only side fix; real-device flows hit the same dialog).
- `test-app/src-tauri/Cargo.toml` — try a newer `tauri-plugin-deep-link` if released, or pin to a known-good version.
- `test-app/src-tauri/capabilities/default.json` — add `deep-link:allow-is-registered` (separate small gap).
- Inspect Tao's iOS app delegate (vendored under `~/.cargo/registry/.../tao-*`) to see whether `application:openURL:options:` is bridged through to plugins or shadowed.

## Test App: Menu nav migration — remaining flows

Menu nav (`tap on_accessibility_label="menu-toggle"` + `tap on_accessibility_label="goto-X"`) replaces `auto_scroll = true` for non-scroll-testing flows. Already migrated: `dialog_overlay`, `read`, `wait` (scroll_and_tap block), `permissions_lifecycle`, `permissions_grant_revoke`, `deep_link` (first step).

**Still on auto_scroll:** `device_controls` (blocked — see scroll-margin overshoot entry), `webview` (test fails earlier on consecutive-`type` issue, not menu nav). Intentionally on auto_scroll: `scroll.test`, `scroll_search.test`, the `wait_for_elements` block of `wait.test`.

## iOS WebKit Inspector: 0x0 Bounds for Plain Inline Elements

iOS WebKit Inspector enrichment reports 0x0 bounds for `<span>` and certain `<div>` elements that lack explicit dimensions, even when they render visually. Affects:

- Device State labels (`<span>Orientation:</span>`, `<span>Theme:</span>`, etc.)
- ScrollList items (`<div>Item 0</div>`, etc.)
- Likely other inline-styled elements

Symptom: `assert_visible on_text="Item 0"` (no auto_scroll) times out because the viewport filter excludes 0x0 elements. `auto_scroll = true` masks it because the spatial fallback iterates regardless of bounds.

CSS workarounds (display:flex, min-height, min-width on .row containers) tested in `DeviceState.svelte` — didn't change the reported bounds. The fix is upstream in `golem-driver/src/webkit.rs` enrichment: query `getBoundingClientRect()` for inline elements that report empty/zero bounds via the inspector.

**Files:** `golem-driver/src/webkit.rs` (enrichment query).

## iOS sim: first-tap race against fresh-launch WKWebView (parked)

Any flow whose first action after `launch` is a `tap` (or implicit-tap action) flakes on iOS Simulator iPhone 17 (iOS 26.4.1). Confirmed affected: `permissions_lifecycle.test`, `permissions_grant_revoke.test`, `read.test` (when the `+` taps run before any wait). Confirmed unaffected: flows that wait on visible text first (`wait.test`, `dialog_overlay.test`) — the wait absorbs the race.

**Symptom:** the first post-launch `tap` "succeeds" against the matched element bounds (golem-side reports OK because the HID synthesis returned), but the JS click handler does **not** fire. The WKWebView is rendered DOM-wise (CDP enrichment reads the tree fine) but it's *quiescent* — the gesture-recognizer wiring hasn't attached to the iOS sim's HID dispatch path used by XCUITest synthesized taps. Real Cocoa mouse events would wake it up; XCUITest's `IOHID` injection apparently doesn't.

The screen-goes-black artifact is a **secondary symptom, not a consequence of the tap** — once the WebView misses input, iOS sim de-allocates / throttles its GPU surface, and the screen renders black until something forces a wake-up. Manual `xcrun simctl launch + mouse-click` always works because mouse events take the wake-up path.

Mitigations in place (companion + driver):
- `RequestRouter.handleLaunch` now blocks on `XCUIApplication.wait(.runningForeground)` + `windows.firstMatch.waitForExistence` + `staticTexts.firstMatch.waitForExistence` before returning. `/launch` returns only after the DOM has rendered text.
- `RequestRouter.handleTap` / `handleLongPress` / `handleHideKeyboard` root the coordinate on `windows.firstMatch` (forces a snapshot) and use `press(forDuration: 0.05)` instead of bare `tap()` (explicit down/up timing).
- `IosDriver.stop_app` uses `simctl terminate` and adds a 500ms post-kill grace before the next launch (avoids the WKWebView teardown race that otherwise stacks WebViews).

Even with all of the above, flake on `permissions_lifecycle.test` is ~3/5. Pure timing isn't the whole story — the XCUITest HID-injection path takes a different OS route than Cocoa mouse events and fresh WKWebViews drop the former. Fixing it cleanly likely requires injecting touches via `IOHIDEventSystemClient` directly or a private `CoreSimulator` API, which is a significant detour.

**Workaround for flow authors today:** put a `wait on_text="..."` as the first step after every `launch` so the wait absorbs the race instead of the next tap. Note: Maestro and most other mobile test runners have similar limitations on iOS sim privacy + relaunch flows — this isn't a uniquely golem problem.

**Status: parked.** Picking it back up needs a different injection mechanism, not more XCUITest tweaking.

**Files:** `companions/ios/GolemRunnerUITests/RequestRouter.swift`, `golem-driver/src/ios.rs`.

## device_controls.test: iOS scroll-margin overshoot on relational selectors

After menu-nav lands on `#section-device-state`, iOS reports the row's `Portrait` value at viewport y=-27 — the scroll-into-view places the section just above the visible area on iPhone 17 / iOS 26.

`scroll-margin-top: calc(60px + env(safe-area-inset-top, 0px))` in `Menu.svelte` is intended to clear the sticky menu, but the resulting offset under `viewport-fit=cover` overshoots. Affects every assertion in `device_controls.test` that uses `right_of` / `below` relational selectors after menu nav.

**Investigate:** measure the actual sticky-menu height on iOS WKWebView (it may be smaller than 60px on `position:sticky;top:0` with cover-mode), and reconcile with the runtime safe-area inset. Likely fixable by computing `scroll-margin-top` dynamically from the menu's measured bounding box.

**Files:** `test-app/src/lib/Menu.svelte`, possibly `test-app/src/App.svelte`.

## device_controls.test: Android back+launch lands in 37-node partial render

`press button="back"` on Android exits the test-app activity. The next `launch` returns success and `await_first_frame` fires (settle gate passes), but the DOM is 37 nodes — h1 only, no Counter / Menu / sections. Both menu-nav and `auto_scroll = true` time out because the targets don't exist in the tree.

Pre-existing — also fails on the legacy `auto_scroll`-based flow. The Tauri Android WebView doesn't fully re-render after `back` → `launch`. Could be a `singleTask` activity vs WebView lifecycle interaction.

**Files:** `test-app/src-tauri/gen/android/...` activity config; possibly `golem-driver/src/android.rs` settle gate to detect partial render and re-poll.

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

