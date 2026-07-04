# Roadmap

## iOS embedded (non-full-screen) webview support

Surfaced by the embedded-webview fixture in `test-app-b` (a small WKWebView/
Android WebView at a known non-top offset). **Two coupled gaps, iOS-specific:**

1. **golem detects an embedded WKWebView but never reads its DOM.**
   `find_webview_bounds` locates the embedded webview (live run: native frame
   `wv_y=174`), but the WebKit-Inspector enrichment never fires — the inner DOM
   (`wv-alpha`/`wv-beta`/`wv-faint`) never enters the tree, so golem sees an
   opaque native node and can't target/assert/audit anything inside it. Likely
   cause (dig in `golem-driver/src/webkit.rs`): the inspector locks onto the
   primary inspectable page, so a *secondary* embedded webview's page isn't
   enriched. Detection works; enrichment/page-selection is the gap.
2. **`ios.rs` adds `safe_area_top` to the webview y unconditionally** — correct
   for a top-anchored full-screen webview (Tauri: `native_wv_y≈0`), but wrong
   for an embedded webview whose native frame is already screen-absolute
   (`native_wv_y=174` → would be shoved +54pt low). **Latent today** (only
   manifests once #1 is fixed and embedded DOM is actually placed), so it's
   coupled to #1 — fix together: make the inset conditional on the webview
   being top-anchored/cover, else let the native frame carry placement.

Real-world relevant (in-app browsers, OAuth, hybrid screens) but lower priority
than full-screen webviews (handled). **Android embedded WebViews untested** —
CDP enumerates targets, so it may behave differently; check when tackling.
Validation vehicle: a `test-app-b` embedded-webview fixture (was prototyped
this session — a small WKWebView/Android WebView at a known offset — then
reverted; re-add when tackling this).

(The related *full-screen* cover-webview offset — native safe-area inset 54 vs
CSS env 62 → −8pt on every Tauri/full-screen webview — is already fixed:
`webkit.rs` cancels the native inset `ios.rs` added, not the CSS env.)

## Testability: unified I/O seam abstractions (subprocess + HTTP)

Coverage sweep surfaced ~15 functions that directly construct `tokio::process::Command`
or issue HTTP, making their orchestration logic untestable without real devices/network.
Rather than 15 piecemeal injections, do two shared seams:

- **`CommandRunner` trait** (real impl spawns; test fake returns canned `Output`/errors),
  injected where subprocess calls happen: `golem-runner` `installed_state::query`
  (xcrun/adb/defaults), `installer::run_install_script`, `cleanup` shutdown;
  `golem-devices` `lifecycle` (boot/run/spawn), `settings` appliers, `concurrency`
  (adb free-disk), `resource_manager` shutdown; `golem-driver` `android` adb runner;
  `golem-cli` `suite` reboot/wait. A clock/sleep seam pairs with this for the
  reboot/wait timeouts.
- **HTTP/transport seam** for `golem-runner` `actions/external` `handle_http` (inject a
  `reqwest::Client`/transport) and `handle_await_email` (an `ImapPoller` trait),
  `golem-runner` `perf` companion fetch, and `golem-driver` `webkit`/`android` companion
  transport (in-memory duplex for tests).

Also `golem-cli` `install_cache` could take a seam over `installed_state::query` to drive
`evaluate_cache_gates` with fake `DeviceInstallInfo`. Each seam is behavior-preserving
(real impl is the default); the payoff is hermetic unit tests for retry/timeout/error
orchestration. Sizable, architectural — its own session. (The small standalone Cat-3
seams — main color, orchestrator socket_path, stream `impl Write` — were done inline.)

## Scroll: `center` + `visibility_percentage` for edge/partial targets (Maestro parity)

`e2e/cross/scroll_search.test.toml` `horizontal_carousel_scroll` fails (EF408)
**on HEAD** (pre-existing, not a regression): the carousel sits at the bottom edge
of the screen (swipe band ~y=2317). The horizontal swipe stalls — hierarchy node
count doesn't change (`stall 2/2`), `boundary reached`, reverses, stalls again.
Likely causes: the swipe lands in/near the Android system-gesture zone at the
screen edge, and/or the target card is only partially visible so scroll either
can't engage the carousel or prematurely treats a partially-visible match as the
stop condition.

Maestro's `scrollUntilVisible` has two knobs we lack:
- **`centerElement`** — keep scrolling until the target is centered, not merely
  edge-visible. Fixes "found but unusably at the screen edge" and gives the swipe
  a safe interaction band away from system-gesture insets.
- **`visibilityPercentage`** — require N% of the target visible before declaring
  it found, so a sliver peeking at the edge doesn't count as success.

Proposed: add optional `center = true` and `visibility_percentage = N` to the
`scroll` action. `center` scrolls until the matched element's center is within a
tolerance of the container/viewport center; `visibility_percentage` gates the
match. Both default off (current behavior preserved). Also consider insetting the
swipe band away from the system-gesture zone for edge-adjacent containers. Add
unit tests for the centering/visibility math and an e2e once implemented.

## Android Unicode IME: restore on SIGINT/Ctrl-C

The bundled Unicode IME (`GolemImeService`) is restored two ways: the
in-session primary (`ime::restore_all` at suite teardown) and next-run
self-heal (`ime::self_heal` at device init, backed by the `.golem/`
original-IME record, with `ime reset` as the backstop when the record is
gone). The gap: a Ctrl-C **mid-run** skips `restore_all`, so golem's
(invisible) IME stays the default until the next golem run self-heals it.
Functionally safe — the device is never permanently stuck, ASCII typing
still works, and self-heal + `ime reset` recover it — but the keyboard is
wrong in the meantime.

Fix: a SIGINT handler that calls `ime::restore_all` before exit (pairs
with any other teardown the handler should own). Lower priority than the
self-heal path, which already guarantees eventual recovery.

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
hangs past the driver's HTTP timeout (~11-12s), returning EX000 and
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

Combined effect: `/hide-keyboard` EX000s become a single-flow soft
failure rather than a multi-flow cascade. Reboot becomes the rare
escalation, not the routine fix.

**Files:** `companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java`
(`handleHideKeyboard`); `golem-driver/src/android.rs::hide_keyboard`;
recovery glue in `golem-runner` if not already in place.

## Inner-container scroll: status after 2026-06-17 investigation

The original report here described a "33-attempt absorber thrash" locating a
`within` container plus no forward progress inside it (`scroll to "Item 25"
within {below "Scroll List"}`, Android EF408 120s timeout, iOS ~73s slow). That
was **suspected, never confirmed**. A full investigation against
`e2e/cross/scroll.test.toml` (`scroll_within_list`, Item 45) and a fresh
off-screen repro (`scroll to "Item 25" within {below "Scroll List"}` with no
warm-up) found:

- **The thrash does NOT reproduce on current code** when the app DOM is loaded.
  Phase-1 locate finds the container in **1 attempt**; phase-2 scrolls inside and
  finds the deep item in **~4 swipes / ~13s**. The locate-loop hardening
  (dwell-before-lift, overshoot guard, dynamic-start) already landed and absorbed
  the common case. Coverage is real (target resolves in the *visible* tree — see
  [Visibility model](architecture.md), the load-bearing invariant: visible tree
  judges, full tree only hints).
- **Container resolution works** on the test layouts. Ground-truth diagnostic:
  `within = {below "Scroll List"}` returns the real `overflow-y:auto` `<div>`
  (the items' container) as the first candidate — *NOT* an oversized wrapper.
  (Earlier "wrong container" reads were a device-px vs CSS-px mistake: 480dpi =
  3.0×, so the 787-device-px box = ~262 CSS px = the `max-height:300px` list.)
- **The EF408 *did* reproduce** — but only when the Tauri **webview barely
  rendered** (`{2 trees, ~8 nodes}` vs 211 when loaded): a sparse DOM made
  `within` locate find nothing → `container=None` → page-scroll thrash →
  timeout. The real Android EF408 was a **webview-readiness race**, not a
  scroll-algorithm bug — **FIXED** (the post-launch settle gate is now
  webview-aware: it waits for the webview DOM subtree to hydrate, on an
  extended deadline, and surfaces a "webview DOM not ready" launch warning if
  it never does, instead of proceeding on a sparse tree).

**Done this session (committed):**
- Renamed `scroll.rs` locals `*_fp` → `*_fingerprint` (full/horizon), pure local.
- Fixed misleading scroll **labels**: human stream said `strategy N` (always
  "1" for container scrolls, meaningless) and `inner scrollable → strategy {+2}`.
  Added `container: bool` to `golem_events::SubstepEvent::ScrollAttempt` +
  mirror `SubstepDetail` + a `ScrollAttemptResult::ContainerAdvanced` variant.
  Now container scrolls log `[scroll] ↓ container (…)→(…) → container advanced`
  and page scrolls log `preset N`. Renamed `scroll_strategy_switch` →
  `scroll_preset_switch` in human output. junit shows `container` vs `preset=N`.
  Touched: `golem-events/src/lib.rs`, `golem-runner/src/scroll.rs`,
  `golem-report/src/{lib,stream,toon,junit}.rs`. Verified live on Android (phone)
  + iOS (iPad).
- Wrote the [Visibility model](architecture.md) section + an AGENTS.md pointer
  (the visible-tree-judges / full-tree-hints invariant).

**What actually remains** (the rest spun into the entries below):
1. **Relational-selector fragility** — `within` picks `.first()` of *everything*
   below the anchor; works here only by pre-order luck + geometric overlap. See
   "Relational selector overhaul".

**Files:** `golem-runner/src/scroll.rs`, `golem-runner/src/actions/interaction.rs`
(`handle_scroll` `within` resolution), `golem-runner/src/resolution.rs`
(`scroll_swipe_bounded`), `golem-element/src/selector.rs` (relational filters).

## Relational selector overhaul — core DONE 2026-06-17; follow-ups below

**Shipped** (`golem-element/src/selector.rs`, `golem-parser/src/lib.rs` +
`mixin.rs`, `golem-runner/src/resolution.rs`): added `contains`/`inside`
geometric predicates; directional filters (`below`/`above`/`left_of`/
`right_of`) now require **cross-axis overlap** (≥1px) with the anchor; survivors
are **sorted** containment-tightest → proximity-nearest (primary-axis gap) →
tree-pre-order tie-break, then `.first()`. Unit tests in `selector.rs` (overlap
exclusion, nearest-first, contains smallest-enclosing w/ self-exclusion, inside,
containment-beats-proximity). E2e `e2e/cross/selectors.test.toml` covers every
selector facet (text, accessibility_label, index, 7 traits, all 4 directionals,
contains, inside, nested anchor) — green on Android + iOS. `scroll.test`
re-verified on iOS (no regression to `within={below}`). Design rationale (why
geometric not Maestro's DOM `childOf`; scrollability is a hint not a filter) is
preserved below for context.

**Done since (this session):** `input`/`toggle` traits removed (f2da21d, unused +
webview-incompatible); `docs/selectors.md` added; test-app made responsive
(two-column on tablet) and a **Selector Grid** section added (`SelectorGrid.svelte`
— A1..D4 cells + WIDE/TALL/DUP/DIS/checkbox + a `tapped:<label>` readout) driving
a comprehensive `e2e/cross/selectors.test.toml` (text, accessibility_label, index,
glob, traits, all 4 directionals + overlap-exclusion + nearest-first, nested AND
**chained** relational anchors, contains/inside, enabled/checked/clickable,
no_text). Green Android + iOS phone/tablet. The deliberately-fragile case and the
tablet cross-column proof are both covered by the grid now.

**Follow-ups still open:**
- **A2 (tap/swipe centroid) — decided NOT to do.** Tapping the resolved element's
  centre is the correct, predictable contract; if the centre is dead space, the
  author should select the actual child (`contains`/`inside`/relational) or use
  the `x`/`y` offsets. Auto-redirecting to a child centroid is surprising magic.
- **Occlusion by sticky/overlapping elements — DETECTION+ROUTING DONE (webview + native); severity is the follow-up.**
  Taps hit-test sample points (stored as `Element.hit_points`) and route around occluders
  to a clear point; routed coord shows neutrally in `--verbose` `element_resolved` (no
  warning tag — routing isn't a warning). Webview uses `elementFromPoint`; native uses a
  host-side geometric hit-test against the tree's paint order (Android `getDrawingOrder`
  for elevation, iOS tree order), with an `encloses`-exclusion so Compose's coincident
  label/clickable nodes aren't treated as occluders. Heuristic → "may be occluded", never
  blocks. Live-validated on **both** platforms via test-app-b's centre-overlay fixture
  (`occ-button`/`occ-overlay`, in the Compose `MainActivity` and the SwiftUI `ContentView`)
  and `e2e/cross/native_occlusion.test.toml` (ios + android) — a naive centre tap hits the
  overlay, the occlusion-aware tap routes to a clear edge. **Finding:** Android's a11y
  already prunes nodes whose bounds are fully occluded (covered text disappears) and may
  trim an interactive's reachable region — so the host hit-test mostly earns its keep
  where the platform keeps a covered element at full bounds (and on iOS, whose snapshots
  retain occluded elements).
  - **Severity (warn/error) — DONE.** The shipped a11y audit surfaces occlusion as
    `occluded_element` (Warning): it consumes the `hit_points` reachable-fraction ground
    truth (level-dependent floor — strict flags >25% covered, relaxed/critical >50%),
    governed by the level + `a11y_max_errors/warnings` model. Its sibling
    `overlapping_interactive` stays bounds-based; refining it with `hit_points` was
    considered and **dropped** — `occluded_element` already covers the "is it actually
    covered" signal, and a bounds overlap-area threshold is the cheaper lever if it ever
    proves noisy (`HitPoint` carries no occluder identity, so attributing the overlap is
    fuzzy anyway).
  (System-status-bar occlusion of the menu button is a *separate* layer — see "Android:
  sticky menu tap target only half-clickable" below; the hit-test can't see the OS bar.)
- **Install-cache may miss a test-app component edit.** Adding the DIS button +
  checkbox to `SelectorGrid.svelte` didn't reinstall until `--rebuild` (a non-
  rebuild run served the stale app — `on_text="DIS"` EF404, then `--rebuild`
  resolved it at +3 nodes). Investigate whether the source-fingerprint covers all
  test-app files / nested component edits. Low-frequency but causes confusing
  ghost failures; analogous to the companion stale-build trap.
- **`SelectorGroup` has no `deny_unknown_fields` (low priority, future).** A
  typo'd or misplaced selector key (`contais = …`, a count on a non-`contains`
  anchor, etc.) is **silently ignored** by serde rather than rejected, so the
  step quietly does the wrong thing. Adding `#[serde(deny_unknown_fields)]` to
  `SelectorGroup` (and peers) in `golem-parser` would turn typos into clear
  parse errors. Surfaced while adding `contains.min_matches` (which sidesteps
  the issue via a dedicated type). Not urgent — no current breakage — but a real
  authoring footgun worth closing some session.

---

### Original design notes (rationale, retained)

**Motivation.** `within = { below = "Scroll List" }` (and relational selectors
generally) resolve via `find_elements`: a selector with no own-criteria
(text/label/trait all `None`) matches **every** element, then `below` retains
those with `y > anchor.bottom`, and the caller takes **`.first()`** in tree
pre-order. It lands on the right scrollable today only because pre-order happens
to place the container first and it geometrically overlaps. **Latent bug:** wrap
the scrollable in one more non-scrollable `<div>` (extra siblings / uneven
padding) and `.first()` picks the wrapper; a `tap`/swipe at its geometric centre
can then hit dead space. Selectors are deliberately uniform across
tap/assert/swipe, and "is this scrollable?" is **not** a usable signal — a
`<canvas>` scrolls without the flag, an empty `overflow:auto` has it but isn't
scrollable, and the human can't *see* scrollability anyway. So scrollability may
only ever be an internal *hint/tiebreak*, never a filter (consistent with the
[Visibility model](architecture.md): coords/visible-bounds judge, not DOM/CSS
metadata).

**Decision (aligned 2026-06-17): geometric predicates, not DOM structure.**
Maestro's `childOf`/`containsChild` ([relational selectors](https://docs.maestro.dev/reference/selectors/relational-selectors))
were rejected — they make the test reason about a DOM tree the human can't
perceive, the opposite of golem's "test like a human" thesis. Geometric
containment (`contains`/`inside`) is pure coords/dims = how a human localizes
things spatially ("the thing inside that box"), independent of DOM. Honest
caveat to document: a border-less, same-background container isn't visually
perceptible — `contains` is "what a human infers from where the content sits,"
fine for scrolling, shakier for an exact-bounds assertion.

**New selection model** (`golem-element/src/selector.rs`
`apply_relational_filters` + a new sort step; `Selector` gains `contains`/
`inside`; `golem-parser` `SelectorGroup` + `build_selector_from_group` in
`golem-runner/src/resolution.rs` gain the fields):

1. **Filter (set intersection over pre-order matches):**
   - Directional (`below`/`above`/`left_of`/`right_of`): keep half-plane match
     **AND require cross-axis overlap** with the anchor — `below`/`above` need
     horizontal (x) overlap; `left_of`/`right_of` need vertical (y) overlap.
     Threshold = **any positive overlap (≥1px)**. Transparent for the common
     full-width anchor (overlaps everything → no change); only bites for narrow
     anchors (e.g. a left-column heading on a tablet correctly ignores a
     right-column element). A wide element spanning both columns still matches a
     narrow anchor — accepted; author scopes with `contains`/`index` if unwanted
     (no overlap-percentage knob). Anchors still resolved via
     `resolve_visible_anchor` (must be on-screen).
   - Containment (`contains = <selector>` / `inside = <selector>`): keep
     candidates whose bounds fully enclose (`contains`) / are fully enclosed by
     (`inside`) the referenced target's effective bounds.
2. **Sort survivors** by fixed, documented priority (lexicographic, one natural
   key per predicate — keys never cross-contaminate, so a pure `below` never
   cares about size and a pure `contains` never cares about ordinal position):
   - **a. Containment** (if `contains`/`inside` active): tightest first (smallest
     area) — strongest spatial signal.
   - **b. Proximity** (if directional active): nearest first by **primary-axis
     gap only** (not Euclidean) — `below` ranks by vertical gap, etc.
   - **c. Tie-break: tree pre-order** (today's behavior) — keeps `--seed` replay
     deterministic. Genuine ties (e.g. a horizontal row of equal-y icons under a
     full-width heading) fall here intentionally; we do NOT guess "smart" — the
     author disambiguates with `index` or a second predicate.
3. `.first()`.

**`within` then becomes robust:** `within = { contains = { text = "Item 0" } }`
= "scroll inside the box that holds Item 0" → smallest enclosing element = the
list (skips the page wrapper *and* a single item), no pre-order luck.

**Behavior changes to validate (greenfield, allowed, but re-run e2e):**
- `above`/`left_of`/`right_of` change from pre-order to nearest-first (this
  *fixes* them — pre-order gave farthest-first). `below` ≈ unchanged in practice
  (DOM order is top-to-bottom).
- Directional filters now require cross-axis overlap → can return empty where the
  old half-plane matched a far-column element (correct). Empty `within` locate →
  existing "scroll page to bring anchor into view" fallback still applies.
- Re-run `e2e/cross/accessibility_label.test.toml`, any positional flows, and
  `scroll.test`. Add a **deliberately-fragile test-app case** (wrap `ScrollList`
  in a padded wrapper with sibling content) to prove `contains` beats `.first()`.

**Swipe-/tap-centroid tweak (pairs with this):** even with the right region,
aiming a gesture at the raw bounds *centre* can hit padding/dead space. Aim
through the **centroid of the visible child content** (matched item bounds we
already have), not the container centre. Pure geometry, no DOM. Applies to
`tap` on a container and to container-scroll start points
(`container_swipe_start` in `golem-runner/src/scroll.rs`).

**Docs to update in tandem:** `docs/actions-reference.md` (selectors section),
the [Visibility model](architecture.md) cross-reference, and any selector
reference. A new feature SHALL add Rust unit tests (filter overlap math, sort
priority, containment) + e2e.

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

## Step interpolation: wire device/builtin prefixes (`${_device}`, `${self:…}`, cross-device)

`${…}` interpolation is now wired into step execution (`golem-runner/src/interp.rs`,
called per-step in the executor) over the variable store + `${fake:…}` generators,
so `${var}`, `${obj.field}`, `${_loop}` (store-injected) and inline generators all
resolve. But the step-time `InterpolationContext` only sets `store` + `generator` —
it leaves `device`, `device_stores`, `global_store`, `each_vars`, `builtins` as
`None`. So the prefixed/builtin forms `interpolation.rs` supports — `${_device}`,
`${_os}`, `${_udid}`, `${self:var}`, `${global:var}`, `${iphone_17:var}`,
`${_each.x}` — error if used in a step.

Most are tied to features that are themselves roadmap (multi-device flow
coordination, `for_each` over devices), so this wasn't needed for the core var
work. To wire it: build the builtins map (`_device`/`_os`/`_platform`/`_type`/
`_udid`/`_app` from `ctx.device` + the app) and pass `device`/`device_stores`/
`each_vars` into the context the executor constructs in `interp.rs`. Add an e2e
that types `${_device}` / `${_os}` into a field.

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
corrupt when two sims drive XCUITest at once — the EX000 companion drop observed
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
  + `bootstatus`, verified ~17s live.)
- **Post-reboot validation.** After the reboot task clears
  unhealthy, the first flow assigned to the device may hit an
  uninitialised state. Add a sanity probe (single hierarchy fetch,
  expect non-trivial node count, retry once) before marking healthy.
- **Recovery telemetry.** Emit a dedicated `EventKind::DeviceRecovered`
  with the duration + reason so renderers can surface it (currently
  piggybacked on `FlowSkipped`).


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
- **Resolver auto-hide-keyboard fires unconditionally.** Tests that intentionally exercise keyboard-up state will be perturbed. Consider an opt-out flag on the step or scope to specific actions. (Behaviour is now documented in `actions-reference.md`; this entry is only the opt-out feature.)
- **Tests gap.** `find_webview_socket` PID filter, safe-area subtraction, BUTTON/A textContent fallback, `EventLog`, `find_or_allocate_port` Android-only fallback, `ensure_companion_with_reg` UDID cross-check — none have unit coverage.

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
- **Startup-readiness EX000 — FIXED (57b0c97), re-verified.** The companion reported `ready` then its first request dropped (`EX000`) or its first `/hierarchy` timed out (`EF408`) because iOS `/health` returned `ok` the instant the socket bound, with no XCUITest readiness probe (Android already gated on a UiAutomation warm-up; iOS didn't). Fixed by a one-time XCUITest warm-up (screenshot) gating `/health` to `503` until warm. Solo: 50% → **0/10**. Concurrent two-sim re-verify (2026-06-18): once both sims are booted, **0/8** concurrent cold companion starts. **Residual (still open here):** 1/12 device-starts dropped EX000 — but on a freshly **cold-booted** sim, *during launch-settle* (`await_first_frame` `/hierarchy` polling), not at the readiness handshake. That's a transport drop on a settling sim under the boot+launch burst — the deeper process-global / cold-boot contention this entry is about, not the (now-fixed) readiness gap.

The companion-side off-main fix (commit on this entry's removal) prevents one wedge from cascading into all later requests, but doesn't address the underlying issue: XCUITest's HID injection and accessibility-snapshot paths are process-global. When two sims drive XCUITest concurrently from the same host, they interleave on shared `simctl` / `usbmuxd` / `IOHIDEvent` plumbing. Apple's official guidance is one XCUITest run per host process — we're stretching that.

Likely shape of the real fix: serialise the host-side simctl-touching operations (mainly tap synthesis + window-snapshot probes) behind a host-wide mutex, or run each sim's companion in a separate XCUITest process so OS-level state is per-process. The **[[Selective host-wide queue for heavy device ops]]** entry is the natural vehicle — extend it to gate the contended iOS startup/HID/snapshot ops, not just the Android I/O-heavy ones.

Determinism contrast (observed): the iOS-concurrent wedge is **structural** (process-global XCUITest HID + snapshot plumbing). The *startup-EX000* that used to make it recur every run is now fixed (readiness gate, see above) — post-fix it's down to an occasional cold-boot transport drop (1/12). The remaining cross-flow corruption (wrong-field type, backspace stall) is the genuinely structural residue. Android multi-emu failures are **stochastic / load-driven** — `adb` is per-device so there's no structural collision; the trigger is host RAM/CPU/GPU saturation (plus the shared `adb` server) during heavy bursts, hence probabilistic and worse with more emus. Different root *character*, but a host-wide queue/serialisation mitigates both: iOS by serialising the colliding process-global ops, Android by capping the concurrent resource burst.

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

