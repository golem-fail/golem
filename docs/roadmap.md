# Roadmap

## Device-queue scheduling: kill the deadline, separate "stuck" from "queued"

The current hardcoded queue deadline (4 hours after a recent bump
from 20 min — see `suite.rs::setup_device_with_resources`)
conflates two distinct failure modes:

- **Stuck flow**: a running flow is holding the device past
  expectation. Already detectable via the executor's per-flow
  `max_runtime` option — when that fires, the flow aborts and the
  device is freed.
- **Queue wait**: a FlowRun is patiently waiting its turn. A 39-
  test × `--repeat 5` sweep on a single device legitimately
  queues hundreds of FlowRuns, with the tail waiting hours. Not
  a failure — just scheduling.

Today's deadline kills queue-waiters indiscriminately, even when
the queue is making steady forward progress.

**Design:**

1. **Default: unbounded queue wait.** A FlowRun blocks on a
   semaphore tied to free-device count for its slot; no deadline.
   Forward progress is guaranteed by the per-running-flow
   max_runtime circuit breaker.
2. **Optional cap:** `--max-wait <duration>` CLI flag +
   `[options].max_device_wait` in `golem.toml`. Both default to
   `None` (unbounded). Useful for CI with hard time budgets:
   `--max-wait 30m` aborts the queue if the suite is still
   running after 30 min wall-clock.
3. **Concurrency cap follows device count.** Instead of the static
   `ConcurrencyConfig.max_concurrency = 4` racing against actual
   device availability, dynamically cap effective parallelism at
   `min(max_concurrency, available_matching_devices)` once the
   plan phase finishes. Each FlowRun only attempts allocation when
   there's a chance of getting a device — others sleep on the
   device-count semaphore. Eliminates the busy-spin queue.
4. **Out-of-order execution preserved.** Current retry-loop already
   has this property by accident (each retry re-checks all
   devices). The semaphore needs to be device-pool-shaped, not
   slot-shaped, so a flow blocked on a busy iPhone grabs the next
   free iPhone the moment one becomes available regardless of
   queue order.

Bonus: lays groundwork for "boot N identical devices on demand for
`--repeat` parallelism" (already roadmapped as "Boot-on-demand for
`--repeat` identical device pools") — the semaphore expands when
new devices come online.

**Files:** `golem-devices/src/resource_manager.rs` (device-pool
semaphore + ordering rework), `golem-cli/src/suite.rs` (drop the
4-hour deadline, plug into semaphore), `golem-cli/src/cli.rs`
(`--max-wait` flag), `golem-parser/src/{config,lib}.rs`
(`[options].max_device_wait` + parsing).

## Companion: detect + recover wedged UiAutomation handle

**Observed 2026-06-03**: when the host-side `am instrument` driver
process (the one that keeps the companion's `UiAutomation` instance
alive) is killed but the companion process keeps running, the
companion enters a permanent zombie state — `getRootInActiveWindow()`
returns null forever, even though the device and target app are
fine. Every subsequent `/hierarchy` returns 500 "no active window".
Sweep diagnosis showed Pixel 7a's `uiautomator dump` shell command
worked (different IPC path) but the companion's Java call did not.

**Causes that trigger this:**
- User kills the host-side golem with SIGKILL (instrumentation
  child process orphaned without proper teardown).
- `adb` server restart mid-suite (instrumentation channel torn down,
  companion process survives).
- Crash + auto-restart of the companion's parent instrumentation.

**Mitigations:**
1. **Host-side cleanup signal.** When `golem run` exits, send a
   shutdown POST to every active companion (`/shutdown`) so the
   companion process exits cleanly rather than orphaning. Next
   `golem run` re-spawns instrumentation + a fresh companion.
2. **Companion-side staleness detection.** When
   `getRootInActiveWindow()` returns null for >N consecutive calls
   over >M seconds despite the app being foregrounded
   (`activityManager.getRunningTasks` or similar), the companion
   should `System.exit(0)` to trigger instrumentation auto-restart.
   Host re-registers, fresh handle, sweep continues.
3. **Driver-side detection.** When the Android driver gets 500
   "no active window" with `attempts: 3` (our retry payload),
   it could trigger a companion restart via `adb shell am
   force-stop fail.golem.companion` followed by re-`am instrument`
   spawn. Heavier hammer; only useful if (2) can't be relied on.

**Files:** `companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java`
(staleness detector + suicide), `golem-driver/src/android.rs`
(detect persistent "no active window", trigger restart),
`golem-cli/src/registration.rs` (re-register on companion restart).

## Install cache: don't persist `FailedScript` on transient errors

`InstallCache::record_failure((udid, bundle), FailedScript)` is a
one-shot — once set, every subsequent flow targeting that pair
SKIPs with "install_script failed earlier" until cache is wiped
(`--rebuild` or `rm .golem/install-cache.json`). Designed to avoid
wasting hundreds of install attempts on a permanently broken script.

**Gap:** when the transient classifier matches (Mach -308, adb
device-offline, package-service race) and the retry ALSO fails,
the cache still persists `FailedScript` as if the script were
permanently broken. Next flow on the same `(udid, bundle)` SKIPs
forever. For a flake-investigation sweep that's a suite-killer —
two transients in a row poison the cache.

**Fix:** when classification says transient and the retry fails,
DON'T set the cache entry. Leave it empty so the next flow gets a
fresh preinstall (which may succeed now that whatever was transient
has cleared). Only persist `FailedScript` when the failure is
*not* classified transient — i.e. real script breakage.

**Files:** `golem-cli/src/suite.rs::run_install_with_build_coord`
(gate `install_cache.set(..., FailedScript)` on
`!is_transient_install_error(err)`).

## Stub-device end-to-end tests

**The problem.** Unit tests cover individual modules well, but the
end-to-end composition (CLI args → SuiteConfig → in-process server
spawn → client `submit_and_wait` → event stream → renderer → result
files → exit code) has no automated coverage. Real bugs slipping
through:

- `--output toon` silently produced human output (shipped, fixed)
  because `submit_and_wait` always spawned the stream renderer
  regardless of the requested format. Pure composition bug —
  every individual unit was fine.
- Daemon-mode silently skipped writing top-level `results.json` /
  `results.toon` for months (shipped, fixed by routing through the
  same write path). Composition gap: server- vs daemon-mode had
  diverged handlers.

**What stub-device E2E catches that unit tests don't.** Anything that
spans multiple modules:
- CLI flag → wire format → server reconstruction → execution path.
- Per-run output_dir layout under `--repeat`.
- Plan→execute pipeline.
- Coverage strategy fan-out + adaptive stop logic.
- Flake-summary aggregation across `--repeat` boundaries.
- Renderer selection based on `--output`.
- Exit codes for various flow outcomes.
- IPC client↔server contract (config_json field threading, done
  message shape).
- `--trace` boundary capture + sidecar JSON shape.

**Approach.**

1. **Stub driver.** `golem-driver/src/stub.rs` (new) implements
   `PlatformDriver` with deterministic, scripted responses. Extend
   the existing `MockPlatformDriver` (used in unit tests) into a
   fixture that can be driven by a YAML/TOML script:
   ```toml
   [responses]
   "tap on_text=\"Submit\"" = "success"
   "type on_text=\"email\"" = "fail: simulated timeout"
   ```
2. **`--stub` CLI flag.** Hidden behind `#[cfg(any(test, debug_assertions))]`
   or a `stub` feature flag. When set, `SuiteRunner` uses the stub
   driver instead of `IosDriver` / `AndroidDriver`. Bypasses the
   ResourceManager device-boot logic too — stub flows don't need
   real devices.
3. **Test harness** at `golem-cli/tests/e2e/`. Each test:
   - Spawns `golem run` as a subprocess with `--stub`.
   - Captures stdout + stderr separately.
   - Asserts output shape (TOON schema header, JSON structure,
     "Results:" line, flake summary block, exit code).
4. **First test suite to write:**
   - `output_formats.rs` — `--output toon` produces TOON on stdout,
     `--output json` produces JSON, etc. (would have caught today's
     bug).
   - `repeat_flake_detection.rs` — `--repeat 3` with deterministic
     flake script (one pass + one fail + one pass) produces
     "FLAKE 2/3" in summary.
   - `daemon_vs_inproc_parity.rs` — same input via in-process
     orchestrator vs explicit daemon produces identical stdout +
     identical results.json content.

**Out of scope:** anything that needs real device behaviour (HID
injection latency, hierarchy snapshot timing, OS overlays). Those
stay on the real-device sweep path.

**Files:** `golem-driver/src/stub.rs` (new), `golem-driver/src/lib.rs`
(re-export + feature gate), `golem-cli/src/cli.rs` (hidden `--stub`),
`golem-cli/src/suite.rs` (route to stub driver when flag set),
`golem-cli/tests/e2e/*.rs` (new test suite).

## Event-ify remaining server-side eprintlns

**The problem.** Golem runs in two topologies:

- **In-process** (default `golem run` with no daemon): the orchestrator
  server runs inside the same process as the client CLI. Server-side
  `eprintln!(...)` writes to that process's stderr, which IS the
  user's terminal, so messages appear naturally.
- **External daemon** (long-running daemon at `~/.golem/golem.sock`):
  the server is a separate process. Its stderr goes wherever the
  daemon was launched from (background shell, tmux pane, launchd
  log file). The client process's terminal does NOT see those
  messages — they're effectively lost.

Today ~14 setup/diagnostic messages still use server-side `eprintln`,
so external-daemon users silently miss them. Examples:
- `[install] cache load failed ({e}) — continuing with empty cache`
  (`golem-cli/src/suite.rs:360`)
- `[device_settings] {w}` (`golem-cli/src/suite.rs:1217`)
- `[install] failed to write cache: {e}` (`golem-cli/src/suite.rs:1449`)
- `[companion] startup timed out for {platform}` (`golem-cli/src/suite.rs:1760`)
- `[device] Cleanup: {w}` (`golem-cli/src/suite.rs:2337`)
- `[devices] no {target_platform} device found — creating one...`
  (`golem-cli/src/suite.rs:2549`)
- `[registration] error: {e}` / `... registered on port {port}`
  (`golem-cli/src/registration.rs:207, 263`)
- `[install] cache file ... unknown version / unreadable`
  (`golem-runner/src/installer.rs:194, 202, 212`)

**The fix.** Replace each `eprintln!` with `ctx.emit(EventKind::XYZ {
…})` against the suite event channel. Add a matching renderer arm
in `golem-report/src/stream.rs` so the client's human stream prints
the same string. Existing event flow already serialises over the
unix socket to the client, so both topologies produce identical
output.

**Suggested event variants** (one per category, payload tailored):
- `InstallCacheLoaded { ok: bool, message: Option<String> }`
- `InstallCacheWriteFailed { error: String }`
- `DeviceSettingsApplied { warnings: Vec<String> }`
- `CompanionTimedOut { platform: String }`
- `DeviceCleanupWarning { device_id: DeviceId, message: String }`
- `DeviceBootRequested { platform: String, name: String }`
- `RegistrationError { error: String }`
- `RegistrationCompleted { device: String, platform: String, port: u16 }`
- `InstallCacheFileBroken { path: String, reason: String }`

**Exclude from this work** — these intentionally stay as server-side
eprintln (logs visible only to daemon admin):
- `[orchestrator] server — listening on ...` (startup banner)
- `[orchestrator] accept error: ...` (rare server socket error)
- `[orchestrator] read error: ...` (already gated behind `--debug`)
- `[orchestrator] waiting for N active client(s)...` (drain spinner,
  already suppressed for the in-process self-loopback case)

**Files to touch:**
- `golem-events/src/lib.rs` — add new `EventKind` variants.
- `golem-report/src/stream.rs` — add match arms that format each
  variant identical to the current eprintln text (preserve user-
  visible string).
- `golem-cli/src/suite.rs`, `golem-cli/src/registration.rs`,
  `golem-runner/src/installer.rs` — replace each listed eprintln
  with `ctx.emit(...)`. Many sites have an `ExecutionContext` or
  `event_tx` already in scope; for those that don't, thread a
  sender or use `golem_events::channel::EventSender` directly.

**Out of scope:** turning the persistent flake-summary tally into an
event — flake summary is purely client-side aggregation of
already-streamed events.

## Loose-FIFO device queue (multi-tenant orchestrator)

Today blocked FlowRuns are equal-priority — each runs its own poll
loop and whoever's 2s timer fires first after a device frees wins.
Order is essentially random (tokio scheduling). Fine for the
single-client case (all 195 FlowRuns are the same caller), but a
future server-with-many-clients needs loose FIFO so the first client
to submit work has a higher chance of finishing first.

**Design:** on device-release, look up the oldest queued waiter
whose slot shape matches that device, hand it the device. Per-shape
FIFO — a queued iOS FlowRun isn't blocked by older Android waiters
since they couldn't grab an iOS device anyway. Each FlowRun
registers `(arrival_time, slot)` in ResourceManager on entry to the
wait loop and deregisters on allocation/exit.

Combine with **adaptive poll backoff** for the same wait loop:
`Tpoll = 2s + 100ms × waiting_count(slot)`. Keeps inter-poll
responsiveness across the waiter population at ~100ms regardless of
N. Without this, 200 waiters each polling every 2s = ~100 polls/sec
mutex contention; the adaptive backoff caps it at ~10/sec total
under load.

Both pieces need `ResourceManager.waiting_count(slot)` bookkeeping
and a sorted-by-arrival queue per slot shape.

**At ~1000-FlowRun scale**, per-task `discover_all_devices` (the
current mid-sweep-refresh approach) becomes noisy — ~30 adb
subprocess calls/sec. Three escalating options:

1. **Adaptive TTL** — grow refresh interval with waiter count.
   Self-balancing, fully decentralised. Easy to bolt onto current
   per-task loop.
2. **Shared cached snapshot** — one global discover per 30s in
   `ResourceManager`, all waiters read the cache. Constant adb
   load regardless of N. Adds one `tokio::sync::RwLock` field.
3. **Warm-pool cap with cold queue** — cap the number of tokio
   tasks actively polling (e.g. ≤200). Excess FlowRuns sit in a
   `VecDeque` "cold" queue with near-zero per-entry overhead. As
   warm tasks complete, pull next from cold queue and promote to
   warm. Natural FIFO. Bounded mutex contention forever.

Picks compound: (1) is a small tweak; (2) replaces per-task with
shared; (3) bounds the warm pool regardless of both. (3) is the
right end-state for a long-lived server-style orchestrator with
many concurrent client submissions.

**Bonus synergy:** (3) also gives partial loose-FIFO for free — the
cold→warm promotion happens in submission order, so the *entry*
into the active wait population is FIFO. Once promoted, the warm
pool's order is still tokio-scheduled (no guarantee), but at large
N the cold queue dominates the wait time so observed FIFO is
better than today's pure-random ordering.

**Refinements worth considering:**
- **Always-cold entry:** route every FlowRun through the cold queue
  regardless of load (even N=1). Keeps the promotion path exercised
  under small loads so it can't bitrot, and means there's no
  threshold-crossing behavioral discontinuity.
- **Rate-limited promotion:** cap cold→warm at e.g. 1 per 1-2s.
  Sharpens the FIFO ordering (warm pool churns slower, so arrival
  order survives longer) and smooths thundering-herd spikes when
  many waiters arrive at once. Modest benefit on top of the
  natural FIFO.

**Files:** `golem-devices/src/resource_manager.rs` (waiting registry,
on-release handoff), `golem-cli/src/suite.rs::find_available_device`
(step-4 wait loop reads adaptive Tpoll, blocks on a per-shape
signal instead of fixed sleep).

## Device lifecycle: graceful loss

When `discover_all_devices` no longer returns a device we have
allocated (user shut it down mid-sweep, adb dropped the
connection, etc), the FlowRun holding it currently has no
graceful recovery — its driver calls just start failing. Should:
mark the device unhealthy in `ResourceManager`, abort the active
FlowRun with a clean "device disconnected, retry" error, free the
allocation so other queued FlowRuns can shift to alternative
devices.

**Auto-shutdown race.** Devices golem booted itself are tracked
for shutdown at suite end (per `--keep-devices`). If we ever add
an "idle device reaper" (shut down golem-booted devices not used
in N minutes mid-suite), it must coordinate with the allocator
(refuse to shut down a device with queued FlowRuns targeting its
shape).

**Files:** `golem-devices/src/resource_manager.rs` (unhealthy
state + graceful-loss handler triggered by next discover snapshot
showing a previously-allocated device gone).

## Boot-on-demand for `--repeat` identical device pools

`--repeat N` parallelises across devices for free when N matching
sims/emulators are pre-booted, but today golem boots a single device
per platform/shape and serialises repeats on it. To deliver the "5
identical devices = 5 parallel runs" USP without manual pre-booting,
`ResourceManager` would boot N matching sims/emulators on demand when
free RAM permits, capped by `--max-concurrency`. Covered by the
broader "True Parallel Flow × Device Concurrency" entry below.

## Physical iOS screen recording

`xcrun simctl io ... recordVideo` is simulator-only. Real-device
recording requires a different transport — either
(a) `idevicescreenshot` polling + ffmpeg encode (slow), or
(b) USB-Mux QuickTime trace pull (Xcode's approach). Today the
driver `bail!`s when `physical = true` so the failure mode is loud.

**Files:** `golem-driver/src/ios.rs` (physical path).

## `golem trace-extract` subcommand

Subcommand `golem trace-extract <flow> <step>` (or `<flow>
<boundary_ms>`) that pulls a single video frame from a per-block
recording at the matching sidecar-offset. Two impls considered:

- **Shell ffmpeg**: simplest, zero build deps. Fails if ffmpeg
  isn't installed (~not preinstalled on macOS or minimal Linux).
- **Pure-Rust stack** (`mp4` + `openh264` + `image`): ~2.5-4 MB
  added to the release binary; works in any env (relevant if golem
  ever exposes an MCP server). Defer until that use case
  materialises — `--trace` PNGs already give snapshot-time frames
  for the common case.

**Files:** `golem-cli/src/trace_extract.rs` (new subcommand).

## Phase 2 and Phase 3 robustness sweep coverage

Phase 1 covered single-test, single-device runs only (78 entries,
64 at 5/5).

**Phase 3 — suite-context**: substantially exercised in the
2026-06-01 session. The 35-test sweep on Pixel 8 Pro API 36 ran
many times during intermittent investigation. Current ceiling
~98% per sweep (172/175 across 5×). Distinct intermittents that
were chased and either fixed or characterised: alert delivery
race, dialog dismiss race, accept/dismiss internal deadlines,
stylus handwriting overlay corruption, tap-too-long → text
selection, `assert_alert` not polling. Phase 3 isn't formally
"done at 5/5 per test" but the suite-context infrastructure is
proven stable enough to keep using.

**Phase 2 — multi-device** (iPhone+iPad, iOS+Android
simultaneous): not yet run. Surfaces XCUITest cross-flow
corruption (already roadmapped under "iOS concurrent flows") and
Android emulator resource-contention.

Tracking files (`/tmp/golem_robust.{json,log}`) and the `robust.sh`
driver script were transient — re-derivable from the sweep plan.

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

- **`cssSafeAreaInset` invisible to callers.** Today the WebKit Inspector enrichment subtracts the inset locally and discards it. Adding `css_safe_area_top: i32` to `HierarchyMeta` (default 0) keeps the diagnostic record. Sets up Android once an equivalent surfaces.
- **`tap()` → `press(forDuration: 0.05)`.** Pages with a long-press distinguisher above ~50ms threshold may classify these as long-presses. Document the boundary or add an explicit `tap-fast` shorthand.
- **Resolver auto-hide-keyboard fires unconditionally.** Tests that intentionally exercise keyboard-up state will be perturbed. Consider an opt-out flag on the step or scope to specific actions.
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

### `--max-concurrency <N>` — Parallel device limit

Flag is defined but never read. `ResourceManager` uses default concurrency config regardless of this value.

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

Menu nav (`tap on_accessibility_label="menu-toggle"` +
`tap on_accessibility_label="goto-X"`) replaces `auto_scroll = true`
for non-scroll-testing flows.

Most flows are migrated. `device_controls` now uses menu nav for
all navigation; one residual `auto_scroll = true` survives on the
"after press(home) + relaunch, find Theme: again" step (the
relaunch state means the menu may not be where it was). That's
the correct use of auto_scroll, not pending migration.

Intentionally on auto_scroll: `scroll.test`, `scroll_search.test`,
`element_find.test` (Scroll List items are inside an inner
overflow-y:auto container — auto_scroll is the only way to bring
items 1-4 into the outer viewport).

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

