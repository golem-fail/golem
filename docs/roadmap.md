# Roadmap

> **Being migrated to [GitHub Issues](https://github.com/golem-fail/golem/issues).** Anything with a clear **problem, reproduction, and acceptance criteria** belongs in an issue (with Type, scoped labels, Effort, and `blocked by`) — **not** here. This file is a **temporary** holding pen for still-vague ideas that don't yet have a crisp repro/acceptance. Entries graduate to issues as they sharpen, and this file will **eventually be deleted**. Don't add a new entry here if you can already write Problem + Reproduction + Acceptance — open an issue instead.

## Testability: I/O seam abstractions — remaining sites

The shared seams exist (`golem-common::command` `CommandRunner` +
`golem-runner::http_transport` `HttpTransport`, each with a fake + restoring
test guard) and cover device boot/wait, `installed_state::query`, the adb
driver funnel, device reboot/recovery, and the `http` action. Sites still on
raw `tokio::process`/`reqwest`, to wire when hermetic tests are wanted:

- `installer::run_install_script` streaming seam — **migrated to #66** (needs a dedicated
  streaming trait method; its 3 tests still spawn real scripts, nextest SLOW).
- The `screenrecord` spawn in `golem-driver` `android` `start_recording` — same live-child
  shape.
- Lower-value auxiliary sites: `golem-driver` `cdp`/`webkit` (lsof/ps/adb + CDP
  `reqwest::get`), `golem-runner` `perf` (adb + companion fetch), `golem-devices`
  `settings`/`concurrency`/`resource_manager` appliers, `capture` ffmpeg, `fingerprint`.
  Wire opportunistically when a bug there needs a regression test.

(A clock/sleep seam proved unnecessary — `tokio` `start_paused` advances the reboot/wait
timeouts deterministically.)

## Companion `/hide-keyboard` reboot escalation on recurrence

The `/hide-keyboard` wedge is already handled softly (companion caps the
`dumpsys input_method` sequence at a deadline and returns `wedged:true`;
the driver retries once, then treats a persistent wedge as a soft
success). Not yet done: escalate to `adb reboot` when the retry *also*
wedges. `DeviceCompanionWedged` is attached runner-side via `fail_code!`
in `resolution.rs`, not by the driver, so a `hide_keyboard` error doesn't
map to a Device-domain code today — and a persistently-wedged companion
is already caught by the next `/hierarchy` fetch's wedge path (which does
reboot). Wire the explicit escalation only if that backstop proves
insufficient.

**Files:** `golem-driver/src/android.rs::hide_keyboard`; the wedge→reboot
glue lives in `golem-cli/src/suite.rs` (see `[[project_pixel_7a_wedge.md]]`).

## Step interpolation: cross-device & `for_each` prefixes

The single-device builtins (`${_device}`/`${_os}`/`${_platform}`/`${_type}`/`${_udid}`/`${_app}`) are migrated to #40. `${_each.x}` is wired to block-level data iteration (#93): a `for_each = "data"` block binds each `[[data]]` row's fields under the `_each.` prefix. Remaining: the prefixed **cross-device** forms `${self:var}` / `${global:var}` / `${<device>:var}` still error at step time (the step `InterpolationContext` leaves `device_stores`/`global_store` as `None`). These are gated on a feature that must exist first:
- `${self:}` / `${global:}` / `${<device>:}` → **multi-device flow coordination** (planned; see "Multi-Device Flow Coordination").

Wire them into the step `InterpolationContext` when those land. **Files:** `golem-runner/src/interp.rs`.
## Confirm host-queue benefit on a load-saturated host

The selective host-wide queue is built and wired
(`golem-common::host_queue`: `OpClass::{AdbHostIo,Screenshot,Dumpsys,Simctl,Install,CompanionLaunch}`,
`acquire_then_run`, congestion metering, and a `[host-queue] slow permit wait`
tripwire). Same-class heavy ops serialize host-wide; light per-step verbs
(tap/type/swipe/hierarchy) stay parallel. Validated locally for *safety* —
no-harm + correct engagement at 2 and 4 emus (queue engaged, 1.5s dumpsys
wait at 4 emus, no false-timeouts, no deadlock on Android or iOS).

Unconfirmed: the *benefit*. The original 48% (2-emu --trace) regression is
load-driven (adb-server / GPU-encode saturation) and does not reproduce on
local Apple-Silicon hardware even at 4 emus — both queue and no-queue arms
pass 100%, so the same-session A/B can't isolate the fix.

Remaining: run the 2-emu (or 4-emu) --trace sweep with and without the queue
on a load-saturated host (the environment where 48% was seen). If no-queue
reproduces the regression and the queue lifts it, delete this entry. If the
regression can't be reproduced anywhere, the queue stands on no-harm +
sound-mechanism grounds — delete this entry.

Deliberately unbuilt: the step-timeout permit-wait exclusion. Measured wait is
~94ms avg at 4 emus (orders under a step budget) and the tripwire flags a
single wait ≥2s. Build the exclusion only if the tripwire fires in the wild.

**Becomes load-bearing when**: CI needs 10+ concurrent emus. At that
point uncontended bursts would saturate the host fork/IO budget and
this queue becomes necessary, not just optimisation.

**Cross-platform extension (iOS) — partly landed:** the queue's iOS *startup*
side shipped — `OpClass::CompanionLaunch` serializes XCUITest bring-up
host-wide (see "iOS concurrent flows"), which fixed the concurrent-startup
wedge. What remains of this idea is serializing the *per-step* process-global
ops (tap-synthesis + window-snapshot) behind a host-wide `Semaphore(1)` — the
proposed fix for the residual cross-flow corruption. That's deferred and gated
on reproducing the corruption first (it taxes the hot path); tracked in the
"iOS concurrent flows" entry. Note the failure *character* differs: iOS is
structural (process-global XCUITest), Android is stochastic/load-driven (host
RAM/CPU/GPU + shared adb server) — mitigated by capping the concurrent burst.

## Device-queue scheduling: semaphore + concurrency-cap-follows-device-count

Queue wait is now unbounded by default; `--max-wait` opts into a
hard cap. Remaining items from the original scheduling rework:

1. **Concurrency cap follows device count.** Instead of the static
   `ConcurrencyConfig.max_concurrency = 4` racing against actual
   device availability, dynamically cap effective parallelism at
   `min(max_concurrency, available_matching_devices)` once the
   plan phase finishes. Each FlowRun only attempts allocation when
   there's a chance of getting a device — others sleep on the
   device-count semaphore. Eliminates the busy-spin queue.
2. **Semaphore-based wait, device-pool-shaped.** Replace the 2s
   retry loop with a per-pool semaphore so a flow blocked on a
   busy iPhone grabs the next free iPhone the moment one becomes
   available, no polling, no mutex thrash. Out-of-order execution
   is preserved (a queued iOS FlowRun isn't blocked by older
   Android waiters).
3. **`[options].max_device_wait` in `golem.toml`** to complement
   the CLI `--max-wait` flag.

Bonus: lays groundwork for "boot N identical devices on demand for
`--repeat` parallelism" (already roadmapped as "Boot-on-demand for
`--repeat` identical device pools") — the semaphore expands when
new devices come online.

**Files:** `golem-devices/src/resource_manager.rs` (device-pool
semaphore + ordering rework), `golem-cli/src/suite.rs` (plug into
semaphore), `golem-parser/src/{config,lib}.rs`
(`[options].max_device_wait` + parsing).

## Companion: driver-side restart of a wedged UiAutomation handle

When the host-side `am instrument` process is killed but the companion
survives, `getRootInActiveWindow()` returns null forever and every
`/hierarchy` returns 500 "no active window". Two recovery paths already
exist: the host POSTs `/shutdown` to companions at teardown (clean exit,
no orphan), and the companion self-exits when consecutive nulls cross the
staleness threshold — both trigger a fresh instrumentation + companion on
the next run.

Not yet done — a driver-side hammer for a wedge that survives both: when
the Android driver gets 500 "no active window" with `attempts: 3` (our
retry payload), restart the companion via `adb shell am force-stop
fail.golem.companion` + re-`am instrument`. Defer until a wedge is
actually observed surviving the shutdown + self-exit paths.

**Files:** `golem-driver/src/android.rs` (detect persistent "no active
window", trigger restart), `golem-cli/src/registration.rs` (re-register
on companion restart).

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

## Idle device reaper (mid-suite) + auto-shutdown race

Not built. If we add an "idle device reaper" (shut down
golem-booted devices unused for N minutes mid-suite, to free RAM
during long sweeps), it MUST coordinate with the allocator: refuse
to shut down a device that is allocated or has queued FlowRuns
targeting its shape. Devices golem booted itself are tracked for
shutdown at suite end (per `--keep-devices`); a reaper would be the
first mid-suite shutdown path and could race the allocator.

**Files:** `golem-devices/src/resource_manager.rs`.

## Boot-on-demand for `--repeat` identical device pools

`--repeat N` parallelises across devices for free when N matching
sims/emulators are pre-booted, but today golem boots a single device
per platform/shape and serialises repeats on it. To deliver the "5
identical devices = 5 parallel runs" USP without manual pre-booting,
`ResourceManager` would boot N matching sims/emulators on demand when
free RAM permits, capped by `--max-concurrency`. Covered by the
broader "True Parallel Flow × Device Concurrency" entry below.

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

**Impl decision:** ffmpeg is currently used only for non-critical work (a11y), so a shell-ffmpeg `trace-extract` is acceptable if built for that. We will **not** adopt the pure-Rust stack *just* for `trace-extract` — pure-Rust is justified only if/when frame extraction becomes **test-critical** (extracting frames to actually drive/judge running tests, or an ffmpeg-less MCP server). Deferred until such a consumer exists.

## Stale-bundle defense (Tauri iOS build pipeline)

`scripts/install-app.sh` and the corresponding template now (a) clear the per-arch build dir so the `tauri-cli` rename step succeeds, (b) prefer the per-arch path over the xcarchive copy when picking the produced `.app`, and (c) hard-fail when the picked `.app`'s mtime predates the build start. That closes the specific failure mode where weeks-old bundles were silently installed (see post-mortem: "menu missing" was actually "running an Apr 20 build for 3 weeks").

Further hardening that would catch the next variant of this class:

- **Content sanity hash.** Hash `test-app/dist/` after `npm run build` and verify the same hash appears as an embedded resource inside the `.app` (Tauri compresses the web bundle into the Rust binary, so we'd compute the hash on the source dist and embed it as a build-stamp the runner can `grep -F` for). Catches the case where Tauri produces a `.app` with empty/wrong web assets.
- **Reject `set +e` failures with a known signature.** The tolerated `tauri-cli` rename error is "failed to rename app ... Directory not empty". Instead of blanket-tolerating any nonzero exit, parse stderr and only tolerate that exact line. Anything else fails fast.
- **Build cache key includes lockfiles.** `install_cache.rs`'s fingerprint is git porcelain — works when lockfiles are tracked. When they're not (e.g. some downstream consumers), include lockfile hashes explicitly so `cargo update` / `npm install` invalidate the install cache.

## True Parallel Flow × Device Concurrency

Running `golem run a.toml b.toml` on ios+android = 4 device-runs available but only 2 execute in parallel (one per booted device per platform). Other 2 wait for devices to free. Machines with spare RAM could run all 4 at once.

**Desired:** Boot additional simulators/emulators on demand when:
- `total_device_runs > currently_booted_matching_devices` AND
- Free RAM above threshold (per-device ~2-4GB)

**Limits:** `--max-concurrency <N>` always caps — if N is lower than the heuristic allows, N wins. Default stays 4.

**Cleanup:** Track which sims/emulators golem booted (vs user's) so they can be shut down afterwards. Respects `--keep-devices`.

**Note:** Works with the existing install script support — fresh sims booted on-demand will have their install_script invoked automatically via the existing pipeline.

**Files:** `golem-devices/src/resource_manager.rs` (boot-on-demand logic), `golem-devices/src/concurrency.rs` (headroom checks), `golem-devices/src/{ios,android}.rs` (boot helpers + tracking).

## Multi-Device Flow Coordination (Chat Tests)

Some flows use two apps on two different devices that must run together (chat client + chat server). Today's suite model spawns a separate flow task per platform; two devices never coordinate inside one flow execution. The new `FlowRun { slots: Vec<DeviceSlot> }` structure supports 2+ slots, but the initial Plan implementation only emits single-slot FlowRuns.

**Implementation:**
- Plan generator detects apps with incompatible `[[flow.apps.devices]]` constraints (e.g. different platforms) and emits one `FlowRun` with a `DeviceSlot` per incompatible group.
- Execute phase acquires ALL slots' devices before starting the flow; runs the flow with multi-device context (flow steps can `{ action = "launch", app = "b" }` to switch focus between devices).
- Device release happens after the whole FlowRun completes, not per-slot.

**Depends on:** clarification of flow-step semantics across devices — which device is "current" at each step, how `{ action = "launch", app = "b" }` switches focus, how assertions scope. The slot infrastructure already exists; the missing piece is step-level semantics.

## Transient Install Errors: Retry Classifier Polish

`golem-cli/src/suite.rs::is_transient_install_error` classifies a small set of known-recoverable install-script error patterns and retries the script once with `install_only=true` (reusing the already-built artifact). Currently matches:

- `Mach error -308 (ipc/mig) server died` / `NSMachErrorDomain code=-308` — CoreSimulator IPC blip on freshly-booted iOS sims
- `error: device offline` / `error: device not found` — adb device-state race during emulator early boot

**What's left:**
- iOS `bootstatus` grace probe to eliminate the Mach -308 case at source — **migrated to #67**.
- Expand the classifier as new transient patterns surface in CI logs. Conservative — adding patterns that aren't actually recoverable just masks real errors behind a 3s delay.

## iOS concurrent flows: cross-flow focus / state corruption

Single-device runs are stable; iPhone + iPad in parallel is where the tail lives. The two worst failure modes are now fixed; the remainder is the hard saturation/corruption tail.

**Fixed — infra exists, don't rebuild:**
- **Concurrent-startup wedge** → `OpClass::CompanionLaunch` serializes iOS XCUITest bring-up host-wide + a startup deadline (`golem-common/src/host_queue.rs`, `golem-cli/src/suite.rs`). Two sims launching `xcodebuild test-without-building` at once no longer collide and hang.
- **Companion-death ED404 cascade** → bounded companion-restart recovery in slot setup: a companion that goes unreachable mid-suite is relaunched (2 attempts) instead of failing every queued flow ED404. 2026-07 cross-platform sweep: ED404 10→0, suite 7/28→21/28. Registration deadline is tiered — `COLD_REG_DEADLINE` (90s, fresh install) vs `RELAUNCH_REG_DEADLINE` (25s, restart of an installed companion) — so a chronically-dying companion fails fast instead of burning ~a minute per restart (bounds the recovery tail).
- **Cold-start `/hierarchy` warm-up** → `handleLaunch` does one throwaway `HierarchySerializer.serialize` after activate, behind the launch gate, so the first real `/hierarchy` hits a warm accessibility-snapshot path (the `/health` screenshot warm-up only attaches the screenshot subsystem). App-scoped, no SpringBoard query.

**Still open (the genuine tail):**
> The runtime host-headroom throttle lever (adaptive backpressure on rising companion round-trip latency) is now tracked as a feature in **#63**. The remaining sub-modes below stay here until they sharpen.
- **EX000 was a catch-all — now split into coded errors** (`golem-driver/src/common.rs` classifies companion request failures). Connection-refused → `D505` (companion unreachable: death or cold-start drop); a `504` / client-timeout → `D503` (companion wedged, alive but stuck). Only genuinely-unattributable transport errors stay `EX000`. This makes the sub-modes *measurable* — a prerequisite for the fixes below. The failure modes themselves remain:
  - *cold-start `/hierarchy` drop* (now renders `D505`) — the `handleLaunch` warm-up targets it; isolated benefit unproven (confounded: freshly-**booted** sims sit in the settle window and drop MORE than long-warm sims, independent of the warm-up).
  - *mid-flow companion death* (`D505`) — the XCUITest host exits mid-flow under load. Restart-recovery catches it at *setup* time, but a death *during* a flow still fails that flow. Needs mid-flow death detection + retry.
  - *main-thread wedge* (`D503`, via the companion's `504`) — companion alive but a main-thread call stuck (HID/snapshot on a saturated host).
  Real levers: **cap concurrency to host headroom** (stop over-subscribing — the dominant driver) and mid-flow companion-death retry. Not the warm-up.
- **EF408 under host saturation — two distinct causes, don't conflate.** Cross-platform (iOS *and* Android), and the sweep breakdown shows it's **not action-specific** (assert_visible 11 / type 7 / scroll 5 / tap 4 of 25 failing steps; SLOW steps split ~50/50 auto_scroll vs not) — so a per-action timeout bump is the wrong fix. Within a failing flow the cheap steps stay at baseline (~0.5s) right up to the failure — **no gradual per-flow ramp**, so the pressure is *cross-flow* (other flows saturating the host), not this flow degrading. The two causes:
  1. *Scroll non-convergence* — `auto_scroll` loops (each iteration = scroll+hierarchy round-trip) thrash on inner-scrollable/edge targets and blow even a 40s budget. This is a real engine gap, **independent of load** (load only multiplies the iteration cost). Fix under #18 + "Inner-container scroll". Highest-leverage for this half.
  2. *Genuine host slowness* — companion alive but serving slowly under 4–5 concurrent sims/emus; ordinary steps time out. Restart doesn't help (not dead).
  Levers for (2): **cap concurrency to host headroom** (the dominant driver; note `--max-concurrency` is currently a no-op, #24). golem only adapts to load at **device allocation** (`ResourceManager` RAM gate, coarse/upfront) — there is **no runtime throttle**. A principled runtime signal that isolates *host* slowness (not app-slow or http-endpoint-slow): **companion round-trip latency** (e.g. `/hierarchy` fetch time) rising across flows → grant a **bounded** adaptive grace on step deadlines and/or backpressure dispatch. Never open-ended.
  Note: **ED505 (companion death) is the severe end of the same saturation** — a companion pushed past slow into OOM/kill. So headroom capping shrinks the whole EX000/EF408/ED505 tail, not just EF408. (Cheap future refinement: split ED505 into death-after-serving vs cold-start-before-serving by tracking whether the companion ever answered, to *measure* the load share.)
- **Cross-flow state corruption (structural).** Wrong-field type (keystrokes for `Password` landed in `Search` — focus snapshot lagged a step) and a step-6 backspace stall, seen only under concurrent load. Root: XCUITest HID + accessibility-snapshot paths are process-global on the host. Real fix would be a new host-wide `OpClass` serializing the tap-synthesis / window-snapshot ops, OR one XCUITest process per sim. NOT attempted — no fresh evidence it is the active failure mode (recent sweeps were dominated by startup + saturation, now addressed). Gate any HID/snapshot serialization on actually reproducing wrong-field corruption first, since it taxes the per-step hot path.

Android multi-emu contention is the same *character* (host saturation → stochastic drops), mitigated by capping concurrency, not op serialization.

**Files:** `companions/ios/GolemRunnerUITests/RequestRouter.swift`, `golem-driver/src/ios.rs`, `golem-cli/src/suite.rs` (companion restart + launch serialization).

## Distribution: remaining work

The prebuilt-binary pipeline ships for macOS arm64 and Linux x86_64 + arm64
(static musl) — see [distribution.md](distribution.md). What's left:

- **`setup-golem` Action** — extend the binary download to the Linux tarballs.
- **Real Linux device/emulator e2e** — can't run on the macOS dev host.
- **Fuller Linux resolver** — per-flow iOS-leg skip + a `strict_coverage` error
  mode (reusing the existing skip machinery), if the current `--platform android`
  default's iOS-only-flow handling proves too blunt.
