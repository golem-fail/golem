# Roadmap

## Android type can't do Unicode — and HANGS on it (uses `input text`)

`CompanionServer.handleType` types via `executeShell("input text <line>")`
(splitting on `\n`, KEYCODE_ENTER between lines). `adb shell input text` is
**ASCII-only** — the same mechanism behind Maestro's #146. Worse than Maestro,
though: a non-ASCII line doesn't drop quietly, the shell call **hangs**, so the
`/type` HTTP request never returns and the driver times out. Verified: typing
`hello\nこんにちは`, the ASCII "hello" + Enter went through, then `input text
こんにちは` hung → 80s `EF408`. So golem has **no Unicode advantage on Android**
— it's a parity limitation with Maestro, not a differentiator. (No talk claim
should say otherwise.)

First, regardless of the fix: **bound the shell call** so a non-ASCII `input
text` can't hang the handler (the 80s timeout is the worst symptom).

**Recommended fix — bundle a Unicode IME (set-once + broadcast).** This is how
Appium does Android Unicode (`unicodeKeyboard` / `io.appium.settings`), and it's
the only option that works **universally** — native *and* WebView — because the
IME commits text via `InputConnection.commitText()` (the standard input path),
so a focused HTML input in a WebView or a native EditText both receive it. Full
Unicode + emoji. Bundling it gives golem a real "just works, even Unicode" edge
over Maestro (whose users wire ADBKeyBoard by hand).

Design:
- **Where:** add an `InputMethodService` + `BroadcastReceiver` to the companion's
  **main** app (`companions/android/app/src/main`) — already installed alongside
  the instrumentation, so no extra APK. IMEs must live in a normal (non-test)
  package to be discoverable.
- **Activate (set-once per session):** record the current default IME, switch to
  golem's (`ime enable` + `ime set`), then route all `/type` through a broadcast
  (`am broadcast -a GOLEM_INPUT --es text "…"`). Keep `input text` only as an
  ASCII fast-path if desired.
- **Teardown — layered, so the device's keyboard is always restored:**
  1. **Primary (host):** golem restores the original IME at flow/session teardown
     (incl. on SIGINT/Ctrl-C if caught). Host owns the **durable** record.
  2. **Fallback A (companion):** golem passes the original IME id to the companion
     when it switches; the companion holds it **in memory** and restores on its
     graceful self-shutdown (the 5h-inactivity exit and the `/shutdown` POST).
     Covers "host died but companion still alive." No companion-side persistence
     needed (it'd be wiped on the version-bump reinstall anyway).
  3. **Fallback B (next-run self-heal):** golem persists the original host-side
     (`.golem/`). On the next run, if the current default IME *is* golem's
     keyboard and a stored original exists, restore immediately — covers the
     hard-kill gap (SIGKILL / device reboot / adb restart run no companion code).
- **Corruption guard + backstop:** never record golem's own IME as "original"
  (`if current != golem_ime { capture }`), so a leftover can't poison the record.
  Whenever the original is unknown/corrupt, **`adb shell ime reset`** restores the
  system default — no need to know the user's specific IME. This is the universal
  safety valve.

**Alternatives** (narrower; keep as fallbacks, not the primary): CDP
`Input.insertText` for WebView fields only (golem already has the transport, but
it doesn't cover native); `UiObject2.setText()` / `ACTION_SET_TEXT` for native
fields (Unicode-capable but unreliable on WebView virtual nodes); clipboard +
`KEYCODE_PASTE` (Android 10+ blocks background clipboard writes — brittle).

iOS note: XCUITest `typeText` is Unicode-capable, so iOS likely already works —
verify, then any "Unicode" claim is iOS-scoped until the Android IME lands (after
which it's a genuine cross-platform differentiator vs Maestro's #146).

**Files:** `companions/android/app/src/main/…` (new `InputMethodService` +
`BroadcastReceiver` + manifest), `golem-driver/src/android.rs` (IME
enable/set/restore + broadcast type path), `golem-cli/src/suite.rs` (session
teardown restore + next-run self-heal from `.golem/`), and bound the existing
`executeShell("input text …")` in `CompanionServer.java::handleType`.

## Input-mutation verify for `/type` and `/backspace`

Slow IMEs (some Android devices under multi-flow load) return ok from
`input text` / `input keyevent DEL` before the keystroke has actually
propagated to the focused EditText. Step's `post_settle` then sees the
hierarchy fingerprint is stable (no animation in flight) and returns
fast, so the next `assert_visible` runs against a not-yet-updated field
and times out (EF408). Pattern observed on form_fill, type_text — see
e.g. sweep prior to revert of 0.6.28.

A first attempt added per-50ms polling on `getRootInActiveWindow` inside
the companion's `/type` and `/backspace` handlers, which catastrophically
contended on the companion's single-threaded UI_EXECUTOR and produced
null-bursts that tripped the staleness counter → `System.exit(0)` →
reboot cascade. See `[[feedback_companion_a11y_contention.md]]`.

**Correct shape (sleep + single poll + hint to engine):**

1. **Companion:** in `handleType` / `handleBackspace`, after dispatching
   keystrokes, sleep ~150ms (mimics a human pause; long enough for most
   IMEs to land the change). Then **one** `getRootInActiveWindow` →
   `findFocus(FOCUS_INPUT)` → text compare against pre-recorded value.
   Respond with `{"status":"ok"}` on verified mutation, or
   `{"status":"ok", "text_unchanged": true}` when the single check
   didn't see a change. One a11y call max per handler — won't accumulate
   nulls.

2. **Driver:** parse the optional `text_unchanged` flag from the
   response; surface as `Option<String>` or similar from `type_text` /
   `backspace`.

3. **Runner:** when the hint is set, route the next `wait_for_settle`
   through an extended-budget variant: `SETTLE_TIMEOUT 1500 → 3000ms`,
   `SETTLE_INTERVAL 250 → 500ms`. One-shot: only the immediately
   following settle is extended.

**Why this works:** the IME latency is real and small (typically
50-300ms on healthy devices, occasionally up to 1.5s on slow ones).
A fixed 150ms sleep + 1-call check is enough on the happy path and
under 200ms of wasted wall-clock; the extended settle on the hint
path gives the IME another up-to-3s without burning more a11y calls.

**Files:** `companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java`
(`handleType`, `handleBackspace`); `golem-driver/src/lib.rs`, `android.rs`,
`ios.rs` (trait + impls); `golem-runner/src/actions/interaction.rs`
(handlers surface the hint), `golem-runner/src/resolution.rs`
(`wait_for_settle` variants).

iOS analogue worth considering: XCUITest's `value` property on the
focused element is synchronous against the field, so the race we see
on Android doesn't manifest the same way — iOS impl can stay as-is.

## Companion `/hide-keyboard` resilience + Pixel-class reboot escalation

Pixel 7a-class devices under multi-flow load occasionally wedge on
`/hide-keyboard` — the companion's `dumpsys input_method` shell call
hangs past the driver's HTTP timeout (~11-12s), returning EF000 and
sometimes propagating the wedge to the next flow (the underlying
UiAutomation handle stays stuck even after the HTTP request times out).

**Shape:**

1. **Companion:** wrap the `dumpsys input_method` shell call (and the
   subsequent `input keyevent KEYCODE_BACK`) in a bounded executor with
   a ~5s internal deadline. Return `{"status":"ok", "wedged": true}`
   instead of hanging the HTTP handler when the internal deadline
   fires. Frees the request thread so the response returns; underlying
   IME poll may still be queued in a background thread but doesn't
   block the next request.

2. **Driver:** treat HTTP timeout OR `wedged: true` as a transient
   error; retry once with a short backoff (e.g. 1s).

3. **Driver/runner recovery layer:** on second timeout, escalate to
   `adb reboot` recovery per existing wedge-recovery path
   (see `[[project_pixel_7a_wedge.md]]`).

Combined effect: `/hide-keyboard` EF000s become a single-flow soft
failure rather than a multi-flow cascade. Reboot becomes the rare
escalation, not the routine fix.

**Files:** `companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java`
(`handleHideKeyboard`); `golem-driver/src/android.rs::hide_keyboard`;
recovery glue in `golem-runner` if not already in place.

## ANR recovery over-triggers on ordinary flow failures (e.g. EF408 assert timeout)

The post-flow recovery sweep (`golem-cli/src/suite.rs` ~line 1186) runs for
**every** failed flow (`if report.success { continue }`), then probes
`driver.get_hierarchy()`; if that probe errors it concludes "companion
unresponsive at recovery time" and schedules a device reboot. So a perfectly
normal test failure — e.g. `assert_visible` on a deliberately-absent element
timing out (`EF408`) — engages the recovery path, and on a slow/struggling
device the recovery-time probe times out → spurious `ANR recovery: rebooting
device`. Observed on an iOS-solo demo run: the intentional-failure block's
`EF408` printed the reboot line (the reboot didn't even execute — the
single-flow suite was already finishing).

The recovery probe should only engage for failure codes that plausibly indicate
a device/companion health problem — `DeviceCompanionWedged`, transport/HTTP
errors (`EF000`), hierarchy-fetch timeouts — NOT ordinary flow-logic codes
(`EF408` step timeout, `EF404` not-found, `EF412` assertion mismatch, `EF400`
explicit fail). A test asserting something absent is a normal failure, not a wedge.

**Fix:** gate the recovery loop on the failure *class*: skip the hierarchy probe
+ reboot unless `report.first_failure_code` is in the device-health set (or the
step already carried `DeviceCompanionWedged`). Keep trigger (1) wedge-already-seen
and (2) ANR-dialog-detected; drop the blanket "any failure → probe → reboot-if-
probe-fails".

**Files:** `golem-cli/src/suite.rs` (recovery loop gate ~1186-1235).

## Inner-container scroll: absorber thrash locating container + no forward progress inside it

A demo run hit this hard on both platforms. `scroll to "Item 25" within {below
"Scroll List"}` (an `overflow-y:auto` inner list):

- **Android: timed out (EF408, 120s).** Two pathological phases in the verbose log:
  1. *Locating the "Scroll List" container* took **33 swipe attempts** — nearly
     every dynamic-start logged "preset landed inside absorber bounds", cycling
     `scroll_strategy_switch →1/2/3/4/5` and a direction reversal before the
     container was finally found.
  2. *Scrolling inside the container* never progressed — an endless run of
     `inner scrollable → strategy 2` with intermittent stalls, until the step
     timed out.
- **iOS: succeeded but took ~73s for one inner scroll** (and ~50s for an
  `auto_scroll` assert against the gesture widget's off-screen state labels) —
  same root area, slow rather than stuck.

This compounds the existing inertial-momentum issue (below) with two further
problems. **Suspected cause (from the log + a read of `scroll.rs`, not yet
confirmed):** (a) the dynamic-start / absorber-routing path loops when a `within`
target's container must first be located — each retry re-lands in an absorber and
resets progress; (b) container-scroll stall handling resets the stall counter on
every full-hierarchy change without advancing strategy, then eventually falls
through to a boundary reversal that scrolls the wrong way.

**Shape to investigate:** cap dynamic-start retries before falling through to a
strategy switch (kill the 33-attempt thrash); for `container.is_some()` scrolls,
don't zero the stall counter on every full-fp change (let it accumulate so the
exit/strategy logic fires), and don't reverse direction while the container is
still making forward progress. Pair with the move-phase slowdown from the
inertial entry below.

**Files:** `golem-runner/src/scroll.rs` (dynamic-start retry cap; container
stall + reversal logic), `golem-runner/src/resolution.rs` (`scroll_swipe_bounded`).

**Coverage gap:** no e2e flow reliably exercises a deep inner-container scroll —
the demo flow had to drop its `Scroll List` / `Carousel` blocks because of this.

## Inner-list inertial scroll suppression

The dwell-before-lift scroll swipe (in `golem-runner/src/scroll.rs` via
the multi-touch gesture endpoint) successfully kills *native* fling
momentum for page-level scrolls — visually confirmed: 0 overshoot
events on the post-fix sweep. But scrolling INSIDE an
`overflow-y:auto` inner container still hitches/flings: JS-level
inertial scroll computes velocity from move events independent of
release. Even with the finger held still at release, the inner list's
JS scroll code reads the move-phase velocity and applies its own
momentum.

Affects `scroll within=<inner list>` flows — `scroll.test
scroll_within_list` (Item 45) is the canonical example.

**Shape:** for `within` scrolls, also slow the move phase (not just
the dwell), e.g. stretch the gesture's move portion to 600ms instead
of 300ms while keeping the total around 900-1200ms with dwell. Lower
move velocity → JS-side inertial scroll either doesn't trigger or
adds less momentum.

Alternative: chunk a target scroll distance into several smaller
swipes back-to-back. Each gesture is short enough that JS-inertial
adds little, but accumulated effect reaches the target.

**Files:** `golem-runner/src/resolution.rs::scroll_swipe_bounded`
(maybe add a `within_inner_list` variant or a duration multiplier),
`golem-runner/src/scroll.rs` (caller passes the hint when `container`
is set).

## set_location: drop WebView JS hook, grant permission + real geolocation

`golem-driver/src/android.rs::set_location` currently does two things:
1. `adb emu geo fix lon lat` — sets OS-level emulator location.
2. `eval_in_webview("window.__golemSetLocation(lat, lon)")` — pokes a
   Svelte reactive var in the test app's `DeviceState.svelte` to drive
   the rendered "Location:" row directly, bypassing the geolocation
   permission flow entirely.

The eval path is a WebView-only shortcut. It proves nothing about real
geolocation plumbing (navigator.geolocation, runtime permission, OS
LocationManager → app) and only works for Tauri-style WebView apps.
For native apps the hook silently no-ops, so `set_location` falsely
appears to succeed.

Correct path:
- Companion / driver grants `ACCESS_FINE_LOCATION` at install/launch
  (depends on `AndroidManifest.xml` permission persistence — see the
  separate roadmap entry on that).
- Test app's `DeviceState.svelte` reads `navigator.geolocation.watchPosition`
  (or equivalent) and renders the result.
- Drop the `__golemSetLocation` hook entirely.
- iOS: equivalent — set location via simctl, app reads CLLocationManager.

Once those land, re-enable the `location_controls` block in
`e2e/cross/device_controls.test.toml` (currently disabled with a
pointer to this entry).

**Files:** `golem-driver/src/android.rs::set_location`,
`golem-driver/src/ios.rs::set_location`, `test-app/src/lib/DeviceState.svelte`
(remove `window.__golemSetLocation` hook), `e2e/cross/device_controls.test.toml`
(re-enable location_controls block).

## Suite summary rendering

The end-of-suite block mixes metrics at different granularity without
clear separation:

```
Summary [  73.715s]  1 passed, 1 failed             ← flow-level counts
Results: .golem/results/  (json, toon)

── 0 flakes, 1 fail, 1 stable across 1 runs ──      ← test-aggregate over repeats
FAIL     0/1    webview.test (android/Pixel 7a API 36)
PASS     1/1    webview.test (android/Pixel 8 Pro API 36)
```

`1 passed, 1 failed` is flow-level, but `0 flakes / 1 fail / 1 stable`
is test-level aggregated across repeats. Same line height, no visual
cue that they're different things. The trailing per-(test, device)
table is yet another granularity. The reader has to parse three
different "what does this number mean" contexts in adjacent lines.

Rework for clarity — keep all the info but make levels distinct,
e.g. label sections, indent, or use a divider. Worth a design pass
when there's spare cycles. Not blocking.

**Files:** `golem-report/src/stream.rs` (suite-summary block), maybe
`golem-report/src/human.rs`.

## Human output: flow-level code in non-stream formatter + regression test

The live **stream** renderer now shows flow-level abort codes on the human FAIL
line (`FAIL … EF508` / `EF504`): `FlowFinished` carries a `code` and `stream.rs`
falls back to it when no step owns the failure. Remaining:

- **Non-stream path:** `golem-report/src/human.rs::format_flow` (operates on
  `FlowReport`, which already has `first_failure_code`) still omits the code on
  its `FAILED` line. Render it there too for parity.
- **Regression test:** extract the stream FAIL-line string-building into a pure
  helper and assert the flow-level code is shown (and a step-level code still
  wins) — guards against regression. Stream output is `eprintln!`, so it needs
  the small extract-to-helper refactor to be unit-testable.

**Files:** `golem-report/src/human.rs` (`format_flow`),
`golem-report/src/stream.rs` (extract FAIL-line helper + test).

## Branch loops can't terminate on a counter (`_loop` not wired)

GOTO-style looping via branches is meant to work — a block can `goto` itself
(or another) and the executor re-enters it correctly: block re-entry and
per-block iteration tracking already function, and the human stream even
labels `loop_body (iteration N)`. What's missing is a way to *bound* such a
loop — there's no working loop counter to branch on. (A bare `repeat = N`
block field was never the plan; loops are expressed via branches. The gap is
purely the missing bounded counter.)

- The per-block `_loop` counter (`golem-runner/src/loops.rs` `LoopTracker`)
  is never called by the executor, and `_loop` is never injected into the
  variable store — so `[[block.branch]] if_var = "_loop", gte = N` reads
  nothing, never matches, and the loop falls through to its fallback `goto`
  forever (until `max_steps`/`max_runtime` aborts it). Confirmed live: a
  3-iteration branch loop ran until the `max_steps` guard.
- `set_variable` is listed in `policy.rs` but has no arm in the
  `actions.rs` dispatch, so a counter can't be incremented manually either.

**Fix:** inject the per-block iteration count into the var store as `_loop`
before branch evaluation (wire `block_iterations` / `LoopTracker` → vars), so
`if_var = "_loop", gte = N` works. Optionally wire `set_variable` into the
action dispatch for general-purpose counters. Add an e2e flow for a bounded
branch loop — none exists today (`for_each` is the only loop with coverage,
and it iterates an app's devices, not a count).

**Files:** `golem-runner/src/executor.rs` (inject `_loop` into vars before
`evaluate_branch`), `golem-runner/src/loops.rs` (use `LoopTracker` or fold
into the executor), optionally `golem-runner/src/actions.rs`
(`set_variable` dispatch).

## `fake:` generators only evaluated in fixtures, not flow/block vars

`fake:*` generators (email, person, address, credit_card, geo, …) are wired
into exactly one runtime path: `fixture_loader.rs`, which calls
`golem_vars::evaluate::evaluate_generators` with the seeded RNG when a
`load_fixture` step pulls a `__fixtures__/*.toml`. There they work and are
seed-deterministic.

But variables declared directly in `[flow.vars]` / `[block.vars]` are seeded
as the **raw string** — the executor does:

```rust
child_vars.set_in_scope(ScopeLevel::Flow, key, VarValue::String(value.clone()))
```

with no generator evaluation. So `[flow.vars] email = "fake:email"` stores the
literal text `"fake:email"`, and `${email}` interpolates to `"fake:email"`,
not a generated address. This contradicts `evaluate.rs`'s own docstring
("values starting with `fake:` are treated as generator definitions") — reads
as a wiring gap, not a fixtures-only design choice.

No e2e flow uses `fake:` at all, so neither path has end-to-end coverage
(generators + fixture loader have unit/integration tests only).

**Fix:** run `[flow.vars]` and `[block.vars]` through `evaluate_generators`
(seeded from the flow's RNG, same as fixtures) when seeding the store, so
`fake:` works wherever vars are declared. Then add an e2e flow that declares a
`fake:` var, types it into a field, and asserts a non-literal value — and
verify `--seed` replay reproduces it.

**Files:** `golem-runner/src/executor.rs` (flow/block var seeding — call
`evaluate_generators` instead of storing raw), `golem-runner/src/subflow.rs`
(`prepare_child_vars` path), `e2e/` (new fake-data coverage flow).

## "Save on failure" for recordings + --trace screenshots

Today `--no-record` is all-or-nothing and `--trace` always saves every
per-step screenshot + tree, pass or fail. The block-end `adb pull
video` and per-step trace file writes are the I/O bursts hurting
concurrent-emu sweeps. A "save on failure" mode would keep
instrumentation on (cheap on the device side) but only persist the
artifacts that actually carry signal.

**Partial infra exists:**
- `capture_failure_screenshot` (per-step failure path, already
  shipped) — only fires when a step fails.
- `--trace` always saves every boundary regardless of outcome.
- Recording: `stop_recording` always pulls; no discard path.

**Proposed mode** (e.g. `--save-on-failure` or `--trace=on-fail`):
- Record continuously per block (cheap; HW H.264 encoder).
- At block end: if the block passed AND all its steps passed, **discard
  the video on-device** without pulling (`adb shell rm`); else pull
  as today.
- For `--trace` screenshots + trees: keep the per-step capture path
  but write to a ring buffer (last N steps in memory). On step fail,
  flush the buffer + a few subsequent steps to disk. On block-pass,
  discard.

**Why it matters at scale:**
- CI default usage = recording on, perf on per block, --trace off.
  Today's `--trace` mode is debug-only; the next CI-readiness gap is
  making non-trace runs lighter so they scale to 10+ emus.
- The discard path eliminates the dominant I/O burst on the happy
  path (>95% of blocks in a healthy suite). Bad-block bursts remain
  for forensics — which is the only time you needed the data anyway.

**Caveats:**
- Ring buffer + delayed flush makes the "capture cost is per-step"
  intuition fuzzier — pre-failure steps' captures still happen but
  don't hit disk. CPU cost on device side stays; only host I/O is
  saved.
- For `--trace`-mode forensics where you DO want every step, keep an
  opt-in flag (`--trace=always`) so the current behavior is still
  available when needed.

**Files:** `golem-runner/src/executor.rs` (boundary hook for
discard-on-pass), `golem-runner/src/capture.rs` (ring buffer
implementation), `golem-driver/src/{android,ios}.rs` (`discard_recording`
verb that just `rm`s the device-side file).

## Selective host-wide queue for heavy device ops

Two-emu --trace sweep is ~30pp worse than 1-emu --trace (48% vs 85%
pass). Per-emu wedge rate is identical between configs (~9.3
reboots/hr), so the regression isn't per-emu cost — it's bursts at
block boundaries when both emus simultaneously do heavy I/O ops
(`adb pull` video, screenshot encode/transfer, `dumpsys cpuinfo`).
These bursts push otherwise-fast hierarchy calls past the 5s
wedge-detection ceiling → false-positive recovery → cascade.

The cheap fix: a **per-operation-class semaphore**, host-wide. Only
heavy ops queue; light ops stay parallel.

**Queued (Semaphore(1) per host):**
- `adb pull` (video at block end — biggest I/O burst)
- `/screenshot` companion call (large PNG payload)
- Possibly `dumpsys cpuinfo` / `dumpsys meminfo` (slow device walks)

**Not queued (parallelism preserved):**
- `/tap`, `/swipe`, `/type` — light, must stay concurrent for
  throughput
- `/hierarchy` — small payload, single-emu cost dominates
- `/perf` companion endpoint — already small

**Semantics:**
- Acquire-permit time is excluded from step timeout (separate
  `acquire_then_run(timeout, op)` helper) so a flow waiting its turn
  doesn't burn its budget.
- Optional `reuse_recent` hint: if N flows want the same data
  within X ms, the queue can cache + share (rarely useful today —
  one-flow-per-device — but enables future shared-data patterns).

**Why not host-wide queue for ALL ops:** would defeat per-device
parallelism. Cross-device taps are independent and should run
concurrently; only the I/O-burst class actually contends.

**Validation criteria for keep-or-delete:**
- Re-run the 2-emu --trace sweep with this queue active. If pass rate
  recovers from 48% to ≥80%, queue is justified.
- If it doesn't help meaningfully, the regression has a different
  source — delete this entry and look elsewhere.

**Files:** new `golem-driver/src/host_queue.rs` (per-op semaphores +
acquire_then_run); call-site wrapping in `golem-driver/src/android.rs`
for the queued ops; `golem-runner/src/capture.rs` for screenshot path.

**Becomes load-bearing when**: CI needs 10+ concurrent emus. At that
point uncontended bursts would saturate the host fork/IO budget and
this queue becomes necessary, not just optimisation.

**Cross-platform extension (iOS):** the same mechanism is the natural fix for
the iOS concurrent-flow wedges in **[[iOS concurrent flows: cross-flow focus /
state corruption]]**. There the contended class isn't I/O bursts but the
process-global XCUITest plumbing (`simctl` / HID synthesis / window-snapshot).
Adding those to the queued op-classes (a host-wide `Semaphore(1)` around
tap-synthesis + snapshot probes) would serialise exactly the operations that
corrupt when two sims drive XCUITest at once — the EF000 companion drop observed
running `ios:latest:2`. The failure *character* differs (iOS-concurrent is
deterministic/structural — process-global XCUITest; Android multi-emu is
stochastic/load-driven — host RAM/CPU/GPU + shared adb server), but one host-wide
queue abstraction mitigates both: serialise the colliding iOS ops; cap the
concurrent Android burst.

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

## ANR recovery: iOS + post-recovery validation

Android ANR detection + reboot recovery is shipped. Outstanding:

- **iOS ANR-dialog detection.** `detect_anr` matches Android's "isn't
  responding" text; iOS system dialogs have different shapes (Touch ID,
  location prompts, etc). Wire an iOS-side detection path. (The iOS reboot
  *mechanism* is done — `reboot_ios_device` does `simctl shutdown && boot`
  + `bootstatus`, verified ~17s live.) See also
  [[ANR recovery over-triggers on ordinary flow failures (e.g. EF408 assert timeout)]].
- **Post-reboot validation.** After the reboot task clears
  unhealthy, the first flow assigned to the device may hit an
  uninitialised state. Add a sanity probe (single hierarchy fetch,
  expect non-trivial node count, retry once) before marking healthy.
- **Recovery telemetry.** Emit a dedicated `EventKind::DeviceRecovered`
  with the duration + reason so renderers can surface it (currently
  piggybacked on `FlowSkipped`).

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

## Physical iOS device free-disk capture

`ResourceSnapshot::capture_with_ios_simulator` mirrors the host free
disk into `device_free_disk_mb` because the sim's data dir lives on
the host filesystem. Physical iOS has its own storage — for the
recovery / disk-pressure diagnostic to be accurate, we need a real
device-side query. Options:

- `xcrun devicectl device info storage --device <udid>` — Xcode 15+,
  Apple-shipped, returns JSON with capacity + available.
- `idevicediskusage` (libimobiledevice) — external dep, broader OS
  coverage.

Until a physical iOS flow exists in CI, `capture_with_ios_physical`
should be added and wired in for symmetry with the Android path. Today
recovery messages on physical iOS would silently fall back to host-only
disk info (mirrored to `device_free_disk_mb`) which is wrong for
physical hardware.

**Files:** `golem-devices/src/concurrency.rs` (add
`capture_with_ios_physical`), `golem-cli/src/suite.rs` (dispatch
based on `device.physical`).

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

## iOS companion drops connection mid-flow, SOLO (EF000) — suspected regression

Seen on iPhone 17e (iOS 26) running the demo flow **solo** (`--platform ios`,
one device, no Android concurrency): the companion's HTTP server closed the
connection mid-request during `type on_text="Search"` — `EF000 … connection
closed before message completed`. Because it's solo, this is NOT the cross-device
contention of [[iOS concurrent flows: cross-flow focus / state corruption]] — the
companion is dropping on its own. iOS runs are also markedly slow (per-step 2-5s;
`auto_scroll` 7-50s). Intermittent: reproduced once in two solo runs (the other
completed cleanly in ~98s).

Suspected **regression** from the recent Android-focused work (companion /
off-main / a11y changes). ANR recovery now reboots it correctly (~17s), but the
underlying drop needs fixing, not just recovering.

**Investigate:** bisect the last ~2 weeks of companion / driver changes against a
solo iOS demo run; inspect the iOS companion HTTP server / request router for a
lifecycle/threading regression that closes the socket under load (the
`type`/keyboard path is a recurring trigger). Capture the companion-side log at
the drop.

**Files:** `companions/ios/GolemRunnerUITests/` (HTTP server + `RequestRouter.swift`),
`golem-driver/src/ios.rs`.

## iOS concurrent flows: cross-flow focus / state corruption

When iPhone + iPad run flows in parallel, occasional state leaks between sims:

- **Wrong-field type:** observed once on iPhone 17 — typing for `Password` landed in the `Search` input. The next field's focus snapshot apparently lagged by one step, so `typeText` delivered keystrokes to the previously-focused field instead.
- **Step-6 backspace flake:** one of the two flows occasionally times out at `backspace on_text="golem testt"` — element resolves but the action stalls past the step deadline. Solo runs never trigger.
- **Step-19 auto_scroll for Submit:** scroll loop enters strategy 2 stalls under concurrent load even after our scroll-strategy fix.
- **Companion connection dropped on startup under concurrent load (EF000):** running two iOS *versions* concurrently (`os = "ios:latest:2"` → e.g. iPhone 17e/iOS 26 + iPhone 16/iOS 18) plus an Android emulator, the iPhone 17e companion reported `ready` then its first `/hierarchy` returned `EF000 — connection closed before message completed` (companion HTTP server dropped mid-response during the concurrent install+launch burst). ANR recovery correctly fired and rebooted the sim. **Reproducible** — recurred on a second run, including with `--no-build` (so not an install-race artifact). Two concurrent iOS sims is the most fragile config; one iOS + one Android is stable.

The companion-side off-main fix (commit on this entry's removal) prevents one wedge from cascading into all later requests, but doesn't address the underlying issue: XCUITest's HID injection and accessibility-snapshot paths are process-global. When two sims drive XCUITest concurrently from the same host, they interleave on shared `simctl` / `usbmuxd` / `IOHIDEvent` plumbing. Apple's official guidance is one XCUITest run per host process — we're stretching that.

Likely shape of the real fix: serialise the host-side simctl-touching operations (mainly tap synthesis + window-snapshot probes) behind a host-wide mutex, or run each sim's companion in a separate XCUITest process so OS-level state is per-process. The **[[Selective host-wide queue for heavy device ops]]** entry is the natural vehicle — extend it to gate the contended iOS startup/HID/snapshot ops, not just the Android I/O-heavy ones.

Determinism contrast (observed): the iOS-concurrent wedge is **highly reproducible / near-deterministic** — it's structural (process-global XCUITest HID + snapshot plumbing), so two concurrent iOS sims reliably trip it (it recurred every run, incl. `--no-build`). Android multi-emu failures are **stochastic / load-driven** — `adb` is per-device so there's no structural collision; the trigger is host RAM/CPU/GPU saturation (plus the shared `adb` server) during heavy bursts, hence probabilistic and worse with more emus. Different root *character*, but a host-wide queue/serialisation mitigates both: iOS by serialising the colliding process-global ops, Android by capping the concurrent resource burst.

Not blocking — single-device runs are stable, multi-device retry-flaky. **This is the talk's "multi-device is still flaky" caveat, reproduced.**

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

