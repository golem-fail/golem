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

## `create_if_missing` Honours Slot Constraints

`find_available_device` falls back to `auto_create_device(platform, DeviceType::Phone, …)` when no compatible device exists and `create_if_missing = true` is set in flow options. The hardcoded `Phone` default ignores the slot's `device_type` + `os_version`, so a flow needing an iPad or a specific iOS major will get a phone on latest.

**Fix:** pass `slot.device_type` and `slot.os_version` into the create path. Small, isolated.

**Files:** `golem-cli/src/suite.rs` (auto-create call site), possibly `golem-devices/src/lifecycle.rs` if the helper signature needs widening.

## True Parallel Flow × Device Concurrency

Running `golem run a.toml b.toml` on ios+android = 4 device-runs available but only 2 execute in parallel (one per booted device per platform). Other 2 wait for devices to free. Machines with spare RAM could run all 4 at once.

**Desired:** Boot additional simulators/emulators on demand when:
- `total_device_runs > currently_booted_matching_devices` AND
- Free RAM above threshold (per-device ~2-4GB)

**Limits:** `--max-concurrency <N>` always caps — if N is lower than the heuristic allows, N wins. Default stays 4.

**Cleanup:** Track which sims/emulators golem booted (vs user's) so they can be shut down afterwards. Respects `--keep-devices`.

**Note:** Works with the existing install script support — fresh sims booted on-demand will have their install_script invoked automatically via the existing pipeline.

**Files:** `golem-devices/src/resource_manager.rs` (boot-on-demand logic), `golem-devices/src/concurrency.rs` (headroom checks), `golem-devices/src/{ios,android}.rs` (boot helpers + tracking).

## Install Cache: Build-Once, Install-to-Many

Currently the install cache is keyed per `(device_udid, bundle_id)`: the user's install script runs once per device. For suites that run the same flow across multiple devices on the **same platform** (e.g. 2 iOS simulators), the build step is re-run every time — wasteful, since the built `.app`/APK is identical across devices of the same platform.

**Script-side foundation (already in place):** install scripts accept a 4th positional arg, `$4 = "install-only"`. When set, the script skips its build step and installs the previously produced artifact. Scripts that don't support the flag ignore it and do a full rebuild — backwards-compatible.

**Golem-side optimisation (this roadmap item):**
- Split the cache into two layers:
  - `BuildCache: (platform, bundle_id) → Succeeded | Failed` — tracks whether any device for this platform has already triggered a successful build this suite.
  - `InstallCache: (device_udid, bundle_id) → Succeeded | Failed` — per-device install outcome (current behaviour).
- First device for a `(platform, bundle)` pair: invoke script without the `install-only` flag. Build + install.
- Subsequent devices for the same `(platform, bundle)`: invoke script with `install-only`. Install-only path reuses the previously produced artifact.
- On build failure: still `FailedScript` for the `(platform, bundle)` pair; all devices on that platform skip as before.
- Thread-safety: devices may start concurrently — need a per-`(platform, bundle)` mutex around the first invocation so parallel-starting devices wait for the one that "won" the build.

**Ties in with:** [True Parallel Flow × Device Concurrency](#true-parallel-flow--device-concurrency) — the optimisation matters more once more devices can run simultaneously.

**Files:** `golem-runner/src/installer.rs` (split cache types), `golem-cli/src/suite.rs` (first-build-winner coordination).

## Orchestrator Hardening — Review Follow-ups

Bundle of small follow-ups from the first code review of the Plan → Execute refactor. None are correctness-critical today but worth batching into one clean-up pass:

- **Deduplicate `merge_project_apps` logic** — identical in `golem-cli/src/suite.rs` and `golem-orchestrator/src/plan.rs`. Pick one home (orchestrator), remove the other. Also fixes the wrong-`project_root` use in `parse_and_expand` (line ~795 falls back to `flow_dir` instead of the configured project root).
- **Deduplicate slot-label formatting** — `describe_slot` in suite.rs and `shape_label` in plan.rs render overlapping fields; new `DeviceSlot` fields would need updating in two places. Extract one helper in `golem-orchestrator`.
- **Bounds-check in `build_install_matrix`** — `&flows[run.flow_idx]` panics if a caller constructs `FlowRun`s independently. Replace with safe lookup (`flows.get()`) returning skip or error.
- **Device-constraint filter in `device_matches_entry_constraints`** — today only checks `device_type`. Extend to `physical`, `name`, `accessibility_label` so the preinstall safety net matches the resolver's real filter set.
- **`SuitePlanned` event docs** — add a comment noting the variant carries pre-formatted strings and is consumed only by `stream_human` (accumulator/JSON/TOON/JUnit fall through). When machine-readable plan info is needed, introduce a structured sibling event.
- **`plan_event` lifecycle doc** — `take()`-once contract on `SuiteRunner.plan_event`. Currently correct but undocumented; future callers that invoke `run_single_flow` directly (e.g. test harnesses) will silently emit nothing.
- **`project_lock` key granularity** — per `project_root` today. Unnecessarily serialises unrelated apps that happen to share a project root (monorepo). Change key to `(project_root, script_path)` for true per-build serialization.
- **`compute_device_availability` semantics label** — current "N matches (M booted)" can mislead about parallel capacity. Clarify to "N devices (K parallel-usable)" or similar.
- **Unify install/flow `DeviceId` scheme** — preinstall uses `{platform}/{device.name}`, so do flow events. Install uses the same now but the TOON renderer comment implies plain device name; align naming + docs.

**Files:** mostly `golem-cli/src/suite.rs`, `golem-orchestrator/src/plan.rs`, `golem-orchestrator/src/install_matrix.rs`, `golem-events/src/lib.rs` (doc comments).

## Scheduler Prefers Install-Cache-Hit Devices

When picking a free device for a new FlowRun, `find_available_device` currently returns the first free match. For suites with many same-shape FlowRuns, preferring a device whose `install_cache[(udid, bundle)]` is already `Succeeded` saves re-invoking the install script on a fresh device. Matters whenever more than one matching device is booted.

**Fix:** sort `booted` candidates by cache-hit count across the FlowRun's slot apps before returning.

**Files:** `golem-cli/src/suite.rs` (`find_available_device` + caller access to `install_cache`).

## Coverage Multiplier Syntax (`ios:latest:2`)

Extend device constraint parser to recognize a `:N` suffix on the `os` field. `os = "ios:latest:2"` means "resolve 2 devices matching `ios:latest`". Plan generator emits N `FlowRun` entries for that coverage slot.

**Example:** flow targets `ios:latest:2` on both `type = "phone"` and `type = "tablet"` → 4 coverage checkboxes (latest-ios, previous-ios if any version spec, phone, tablet). Min-cover picks the smallest device set that ticks all boxes; a single "latest-ios tablet" device ticks 2 boxes in one run.

**Depends on:** [True Parallel Flow × Device Concurrency](#true-parallel-flow--device-concurrency) — without boot-on-demand, `N>1` stalls when only 1 booted device matches.

**Foundation:** `FlowRun` struct already carries `multiplier: u32` (default 1) so this is a generator change only.

**Files:** `golem-parser/src/lib.rs` (parse `:N` suffix on `DeviceConstraint.os`), `golem-orchestrator/src/plan.rs` (expand N-runs per coverage slot).

## `os = "any"` Default Semantics

Currently `detect_all_platforms()` in `golem-cli/src/suite.rs` defaults to iOS when no `os` constraint is set. Desired: unset/`any` means "run on any available platform" — pick whichever has devices.

**Use case:** platform-agnostic flows (cross-platform assertions, device-agnostic utilities) shouldn't force iOS-only runs.

**Semantics:**
- `os = "any"` → pick any available platform (prefer booted)
- `os = "any:2"` → 2 devices, any platforms, possibly different
- No `os` field at all → behave as `any`

**Foundation:** `CoverageRequirement` struct should carry an `any_platform: bool` flag; Plan generator branches on it.

**Files:** `golem-parser/src/lib.rs` (parse "any" + absence), `golem-orchestrator/src/plan.rs` (handle any-platform resolution).

## Partial Suite on Install Failure

If pre-install fails for app `X` on device `D`, today's per-flow install check marks `FailedScript` in the cache and any flow referencing `X` on `D` is skipped. But UX could be sharper:

- Dedicated `FlowSkipped` event with explicit cause (`InstallFailed(X, D)` vs other skip reasons)
- Aggregated suite summary line distinguishing "install-dep-skip" from genuine flow failures
- Flows that don't reference `X` proceed normally (already the case)

**Foundation:** `InstallCache` already keyed on `(udid, bundle)`; per-flow skip logic already exists. This is polish.

**Files:** `golem-events/src/lib.rs` (enrich `FlowSkipped` variant), `golem-report/src/stream.rs` + `accumulator.rs` (render distinct skip reasons).

## Cross-Process InstallCache

Orchestrator mode (`golem-cli/src/orchestrator.rs`) lets a second `golem run` offload to the first. Currently each process has its own `InstallCache`, so the second suite rebuilds apps the first already produced.

**Desired:** persist install outcomes across CLI invocations. Second `golem run` queries the orchestrator for `(udid, bundle) → InstallOutcome` before running its own install script.

**Implementation options:**
- Shared-memory backing for `InstallCache` (complex, platform-specific)
- Socket-query path in existing orchestrator IPC: `{ type: "install_cache_get", key: (udid, bundle) }` → `{ outcome: ... }`
- Persist to `~/.golem/install_cache.toml` between runs (simpler; staleness handled by wall-clock TTL)

**Foundation:** existing keying is suitable; `InstallCache` trait already abstracts storage.

**Files:** `golem-runner/src/installer.rs` (trait or backend abstraction), `golem-cli/src/orchestrator.rs` (cache-query RPCs).

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

## TOON Timestamp Representation

Human, JSON, and JUnit outputs already emit wall-clock timestamps (local `HH:MM:SS.mmm` prefix for human; ISO-8601 UTC on every report level for JSON/JUnit). TOON is intentionally left out — its compactness-first design would bloat meaningfully with full ISO-8601 strings (16+ chars) per entry.

**Open proposal:** emit a single suite-level `start` unix-epoch timestamp once, then per-event `delta_ms` relative to suite start. A 30-minute suite = 7-digit delta; easy to reconstruct absolute time from `start + delta_ms`.

Needs a concrete schema decision before implementing. Today TOON emits `duration_ms` only.

**Files:** `golem-report/src/toon.rs` once schema is agreed.

## iOS WebView: Slow Element Resolution Between Consecutive Actions

Consecutive `type` actions on iOS WebView elements are slow — resolving the second input field after typing in the first takes >10s. The DOM tree changes after each keystroke (WebKit enrichment re-fetches), and finding the next element requires waiting for the tree to settle.

Example: `e2e/cross/webview.test.toml` step 7 (`type on_text="Search"`) times out at 10s even though the previous `type` (step 5) completes in ~3.6s. The bottleneck is element resolution, not keystroke delivery.

Possible approaches:
- Smarter settle detection that recognizes when WebView content is still updating
- Cache element positions across consecutive steps when the viewport hasn't changed
- Longer default multiplier for WebView-context actions (requires detecting WebView context)


