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

**Status:** resolver layer fixed. End-to-end still broken — see [Scheduler Ignores DeviceSlot.os_version](#scheduler-ignores-deviceslotos_version) below.

`resolver.rs` used to sort candidates by state preference (booted > shutdown) **before** filtering to `:latest`. With iOS 18 booted and iOS 26 shutdown, `os = "ios:latest"` picked iOS 18.

**Fix shipped:** `resolve_devices` now expands `:latest` / `:latest:N` strings into concrete `platform:major` entries using `resolve_latest()` against the available-device snapshot *before* the greedy loop runs. Version is pinned first; state preference tie-breaks only within the chosen major.

**Files:** `golem-devices/src/resolver.rs` (`expand_latest_in_constraint`), tests added covering highest-version-over-booted-older, within-version state tie-break, `:latest:N`, empty-platform fallback.

## Scheduler Ignores `DeviceSlot.os_version`

**Status:** fixed for the single-slot-per-platform case.

Plan emitted `DeviceSlot { platform: Ios, os_version: Some(Exact(26)), … }` but Execute's `find_available_device` filtered by platform only, picking iPhone 16e (v18) over a v26 match.

**Fix shipped:**
- `SuiteRunner` now stores `flow_paths: Arc<Vec<PathBuf>>` + `flow_runs: Arc<Vec<FlowRun>>` populated by `run_suite` after `plan()`. Multi-flow spawn threads both Arcs into each child runner.
- `run_single_flow_with_resources` looks up the current path's `flow_idx`, and for each platform picks the first matching `DeviceSlot` out of that flow's FlowRuns.
- `find_available_device` takes `Option<&DeviceSlot>`; `compatible` / `booted` / `shutdown` are all filtered via `device_matches_slot` (platform, os_version Exact/Minimum, device_type, physical, name). Shutdown auto-boot now only picks within the filtered set.
- `device_matches_slot` promoted to `pub` so Plan and Execute agree on "matches".

**Known remaining gaps (separate items):**
- Coverage fan-out (`:latest:N`, `os = [...]`, `type = [...]`) still runs once per platform — Plan emits N FlowRuns but Execute consumes one per platform. Tracked by [Coverage Multiplier Syntax](#coverage-multiplier-syntax-ioslatest2) + [Dynamic JIT Scheduler](#dynamic-jit-scheduler--device-reuse-lazy-install).
- `create_if_missing` synthesis path uses `DeviceType::Phone` default; should honour `slot.device_type` + `slot.os_version`.

**Files:** `golem-orchestrator/src/{plan.rs,lib.rs}` (pub `device_matches_slot`), `golem-cli/src/suite.rs` (runner state, run_suite populate, slot lookup, `find_available_device` signature).

## FailureBarrier Across Multi-Flow Concurrency

`FailureBarrier` (`golem-runner/src/barrier.rs`) coordinates devices within a single flow — device A fails at step 7, devices B/C abort at step ≥7.

With multi-flow parallelism the scope needs to stay per-flow: failure on `flow_a/ios` should abort `flow_a/android` but NOT `flow_b/ios`. Current barrier is per-flow, cloned to device tasks — should still work but needs doc clarity and a regression test.

**Action:** Keep barrier per-flow. Document semantics. Add test covering 2 flows × 2 devices where one flow fails; verify other flow completes unaffected.

**Files:** `golem-runner/src/barrier.rs` (doc comments), new test under `golem-runner/tests/`.

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

## Suite Orchestration: Plan → Execute Model

Today's orchestration is implicit and per-flow: each flow parses itself, resolves its own devices, runs its own install, spawns its own companion. The suite has no central view. This makes layering in other roadmap items (multiplier syntax, boot-on-demand, cross-process dedup) painful.

**Plan phase (sync, once at suite start):**
- Parse all flow files; merge with project `[[apps]]` defaults
- Call existing `golem-devices::resolver::MinCoverage` per flow to resolve devices
- Emit `ParsedSuite { flows, flow_runs, install_matrix }`
- `install_matrix` is the union of apps **referenced by some flow**, keyed by `(platform, bundle)` — apps in `golem.toml [[apps]]` not referenced by any flow are dropped entirely

**Execute phase (async):**
- Iterate `flow_runs`, acquire device via `ResourceManager`
- Pre-install only `install_matrix` entries applicable to this device + platform
- Ensure companion (reuse if healthy), execute flow, release device

**Status:** implemented as a new `golem-orchestrator` crate housing `plan.rs` + `install_matrix.rs`. `SuiteRunner` rewired to consume `ParsedSuite` instead of raw flow paths.

**Files:** `golem-orchestrator/src/{plan.rs,install_matrix.rs}` (new crate), `golem-cli/src/suite.rs` (rewire `run_suite`), `golem-cli/src/main.rs` (call `plan()` before `run_suite`).

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

## Dynamic JIT Scheduler — Device Reuse, Lazy Install

**Status:** shipped. First pass executes every `FlowRun` as its own worker and saturates booted devices through the `ResourceManager`. Coverage fan-out (`ios:latest:N`, `os = [...]`, `type = [...]`) now drains properly — Plan emits N FlowRuns, the scheduler consumes all N. Parse failures surface as failed FlowReports (the suite no longer aborts on a single bad file). Further optimisations deferred below.

**What shipped:**
- `execute_flow_run(FlowRun)` is the queue unit; `run_suite` spawns one tokio task per FlowRun.
- Per-slot setup (`setup_slot`) runs lazily inside the worker: `find_available_device` → `preinstall_for_device_scoped` → companion reuse-or-spawn → `try_allocate` wait → health check.
- One suite-level event channel + one registration server, shared by every worker (previously a multi-flow suite opened one registration server per flow).
- `ParsedSuite.parse_failures` carries any flow that failed to read/parse/mixin-expand; each becomes a failed `FlowReport` without blocking unrelated runs.

**Still to do (separate items):**
- **Install-cache-hit preference** when picking a free device. Today `find_available_device` picks the first free match; sorting by `install_cache[(udid, bundle)]` hits would save build-once-per-device in suites with many same-shape FlowRuns. Cheap follow-up.
- **Build-once install cache (§ Install Cache: Build-Once, Install-to-Many)** still benefits the most once multiple devices run simultaneously on the same platform.
- **Boot-on-demand (§ True Parallel Flow × Device Concurrency)** — when queue has pending FlowRuns, all matching devices are busy, and RAM permits, boot another sim. Not yet wired; today's setup boots the single best shutdown sim once, per slot.

**Files:** `golem-cli/src/suite.rs` (`execute_flow_run`, `setup_slot`, `preinstall_for_device_scoped`, rewritten `run_suite`), `golem-orchestrator/src/plan.rs` (`ParseFailure` + `parse_one`).

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

Natural follow-up to [Suite Orchestration: Plan → Execute Model](#suite-orchestration-plan--execute-model). Today `SuiteRunner` lives in `golem-cli/src/suite.rs` and IPC logic in `golem-cli/src/orchestrator.rs`. Both belong in the orchestrator crate once the Execute engine stabilises — cleanly separates glue (CLI arg parsing, output rendering) from core (suite execution, multi-process coordination).

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

**Ties in with:** [Suite Orchestration: Plan → Execute Model](#suite-orchestration-plan--execute-model) — builds on the slots-based `FlowRun`.

## Multi-Device Flow Coordination (Chat Tests)

Some flows use two apps on two different devices that must run together (chat client + chat server). Today's suite model spawns a separate flow task per platform; two devices never coordinate inside one flow execution. The new `FlowRun { slots: Vec<DeviceSlot> }` structure supports 2+ slots, but the initial Plan implementation only emits single-slot FlowRuns.

**Implementation:**
- Plan generator detects apps with incompatible `[[flow.apps.devices]]` constraints (e.g. different platforms) and emits one `FlowRun` with a `DeviceSlot` per incompatible group.
- Execute phase acquires ALL slots' devices before starting the flow; runs the flow with multi-device context (flow steps can `{ action = "launch", app = "b" }` to switch focus between devices).
- Device release happens after the whole FlowRun completes, not per-slot.

**Depends on:** [Suite Orchestration: Plan → Execute Model](#suite-orchestration-plan--execute-model) (the slots struct exists) + clarification on flow-step semantics across devices (which device is "current" at each step).

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

## Event Timestamps in Output

`Event`s already carry `timestamp: Instant` on the wire envelope (not serialized) and every `emit` stamps them monotonically. We don't surface time anywhere in rendered output.

Desired surfacing per format:

- **Human stream (`stream_human`)**: show time-of-day (no date) on key transitions — flow start/finish, step start/finish, install finished, setup-narrative lines. `HH:MM:SS` or `HH:MM:SS.mmm`. Helps skim live output without needing a wall clock.
- **JSON / JUnit**: include full ISO-8601 datetime where it's meaningful — flow start, flow finish, install start/finish, suite start/finish. Consumers doing analytics want a real timestamp, not elapsed ms.
- **TOON**: compactness matters. To be discussed — likely a single suite-level `start` unix timestamp + per-event `delta_ms` relative to suite start. A 30-minute suite is a 7-digit ms delta, versus 16+ chars per ISO-8601 string.

Accumulator already stores per-step `duration_ms` in step reports; this is about **absolute** time, not intervals. Needs to thread `Instant`s into the accumulator alongside existing fields (or capture a suite-start `SystemTime` once and compute absolute times for each event at render time from the stored `Instant`).

**Files:** `golem-events/src/lib.rs` (ensure envelope carries what renderers need), `golem-report/src/stream.rs` (human format), `golem-report/src/output.rs` (JSON/JUnit/TOON serializers).

## iOS WebView: Slow Element Resolution Between Consecutive Actions

Consecutive `type` actions on iOS WebView elements are slow — resolving the second input field after typing in the first takes >10s. The DOM tree changes after each keystroke (WebKit enrichment re-fetches), and finding the next element requires waiting for the tree to settle.

Example: `e2e/cross/webview.test.toml` step 7 (`type on_text="Search"`) times out at 10s even though the previous `type` (step 5) completes in ~3.6s. The bottleneck is element resolution, not keystroke delivery.

Possible approaches:
- Smarter settle detection that recognizes when WebView content is still updating
- Cache element positions across consecutive steps when the viewport hasn't changed
- Longer default multiplier for WebView-context actions (requires detecting WebView context)


