<script>
// Deliberately-bad accessibility fixtures, hidden behind a toggle so the rest
// of the app stays a11y-clean (default off → no extra findings in other
// flows). Fixtures sit at the audit's threshold *breakpoints* so different
// levels report different subsets (e.g. a 32dp target is a warning at relaxed
// but an error at strict; contrast only runs at strict). That's intentional —
// see findings/plan_a11y.md. Run the flow at --a11y strict and --a11y relaxed
// to exercise both. Bad buttons override the global 44px min-height.
let show = $state(false);
</script>

<div class="section">
  <h2>A11y Demo</h2>
  <button aria-label="a11y-demo-toggle" onclick={() => (show = !show)}>
    {show ? "Hide" : "Show"} bad a11y
  </button>

  {#if show}
    <div class="a11y-bad">
      <!-- touch-target breakpoints (min dimension dp): 20 <24 err always;
           32 in 24-44 warn(relaxed)/err(strict); 48 ≥44 good. Distinct labels
           so this only exercises touch_target. -->
      <div class="row">
        <button class="t t20">Tap A</button>
        <button class="t t32">Tap B</button>
        <button class="t t48">Tap C</button>
      </div>

      <!-- duplicate labels (triplicate) — clean size/contrast so only the
           duplicate check fires; dashed connector + repeated marker. -->
      <div class="row">
        <button class="ok">Save</button>
        <button class="ok">Save</button>
        <button class="ok">Save</button>
      </div>

      <!-- overlapping interactive pair (clean otherwise) -->
      <div class="overlap">
        <button class="ok ov ov-a">Alpha</button>
        <button class="ok ov ov-b">Beta</button>
      </div>

      <!-- partial occlusion: a labelled, clean-size button with an overlay
           banner painted over its top ~60%. The DOM hit-test reports those
           points unreachable → occluded_element (only the *fully* covered case
           is pruned; this partial case is the target). Tall enough (>44px) that
           the hit-test samples vertical arms, so the covered fraction < 0.5. -->
      <div class="occ">
        <button class="ok occ-btn">Reachable</button>
        <div class="occ-cover"></div>
      </div>

      <!-- missing label: 44dp icon button (size OK → only missing_label) -->
      <button class="icon"></button>

      <!-- text-size breakpoints (box height dp): 7 <10 warn at both; 11 in
           10-12 warn at strict only. Dark text → no contrast finding. The
           right column is the SAME small glyph but with vertical padding that
           inflates the box past the threshold — the box-height check can't see
           the small text there (a padded blind spot). -->
      <div class="row">
        <p class="txt7">tiny 7dp print</p>
        <p class="txt7 pad">tiny padded</p>
      </div>
      <div class="row">
        <p class="txt11">small 11dp print</p>
        <p class="txt11 pad">small padded</p>
      </div>

      <!-- annotation isolation: each is small-font + padded (the pixel pass
           fires) varying ONE thing, to prove what throws the measurement /
           rectangle off — inverted (light-on-dark) colours, rounded corners,
           drop shadow. -->
      <div class="row">
        <p class="iso inv">inverted small</p>
        <p class="iso round">rounded small</p>
        <p class="iso shadow">shadowed small</p>
      </div>

      <!-- multiple issues on ONE item: 20dp tall (touch err) + low-contrast
           text (contrast err at strict) on the same button. -->
      <button class="multi">Go</button>

      <!-- small-font multi-line block, good contrast: small print wrapped
           across several lines (isolates the small-text case from contrast). -->
      <p class="small-multi">
        tiny multi-line print that wraps across several lines of small text
      </p>

      <!-- contrast breakpoints (strict): ~2.1:1 err; ~5:1 in the 4.5-7 AAA
           band → warn. Last, so the flow can auto_scroll the whole section
           into view. -->
      <p class="c-err">contrast error line</p>
      <p class="c-warn">contrast warn line</p>
    </div>
  {/if}
</div>

<style>
.a11y-bad {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 8px;
  align-items: flex-start;
}
.row {
  display: flex;
  gap: 8px;
  /* keep each item its natural height — default `stretch` would inflate the
     unpadded text boxes to match a taller padded sibling and mask the small
     box height. */
  align-items: flex-start;
}
/* size fixtures opt out of the global 44px min-height */
.t {
  min-height: 0;
  width: 72px;
  padding: 0;
}
.t20 { height: 20px; }
.t32 { height: 32px; }
.t48 { height: 48px; }
/* `ok` = clean size + clean contrast, isolating the structural checks */
.ok {
  min-height: 0;
  height: 44px;
  color: #222;
}
.overlap {
  position: relative;
  height: 48px;
  width: 200px;
}
.ov {
  position: absolute;
  top: 0;
  width: 110px;
}
.ov-a { left: 0; }
.ov-b { left: 70px; } /* overlaps ov-a by 40px */
.icon {
  min-height: 0;
  width: 44px;
  height: 44px;
}
.occ {
  position: relative;
  width: 160px;
  height: 80px; /* >44 → hit-test samples vertical arms too */
}
.occ-btn {
  position: absolute;
  inset: 0;
  width: 160px;
  height: 80px;
}
/* Opaque banner over the top ~60%: intercepts elementFromPoint there (top +
   middle sample rows read as covered, bottom row reachable) and hides the
   button's centred label, so this isolates occluded_element — no contrast/
   text finding on text that isn't visible. */
.occ-cover {
  position: absolute;
  top: 0;
  left: 0;
  width: 160px;
  height: 50px;
  background: #8a8a8a;
}
.txt7 {
  font-size: 7px;
  line-height: 1;
  margin: 0;
  color: #222;
}
.txt11 {
  font-size: 10px;
  line-height: 1.1;
  margin: 0;
  color: #222;
}
/* 2dp vertical padding lifts the box just off the glyph: tiny (7dp text) →
   ~11dp box (in the 10-12 band: missed at relaxed, caught at strict); small
   (11dp text) → ~15dp box (past the 12dp ceiling: missed at both). The glyph
   stays tiny either way — box-height can't see it. */
.pad {
  padding: 2px 4px;
}
/* isolation fixtures: 7dp glyph + padding (box well over the floor → pixel
   pass), each adding exactly one confounder. */
.iso {
  font-size: 7px;
  line-height: 1;
  padding: 8px;
  margin: 0;
  color: #222;
}
.inv {
  color: #fff;
  background: #222; /* light-on-dark, square corners, no shadow */
}
.round {
  background: #e0e0e0;
  border-radius: 12px; /* rounded corners expose page bg in the crop corners */
}
.shadow {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}
.multi {
  min-height: 0;
  height: 20px;
  padding: 0 6px;
  color: #b0b0b0; /* ~2.1:1 */
}
.small-multi {
  font-size: 7px;
  line-height: 1.4;
  width: 160px;
  margin: 0;
  color: #222;
}
.c-err {
  margin: 0;
  color: #b0b0b0; /* ~2.1:1 → AA error */
}
.c-warn {
  margin: 0;
  color: #707070; /* ~5:1 → between AA and AAA → strict warn */
}
</style>
