<script>
  // A dedicated surface for exercising selectors: a 4x4 grid of square
  // buttons (A1..D4), a full-width WIDE button, a TALL button, two
  // duplicate-labelled DUP buttons (for index), and a readout that shows
  // the last tapped label so e2e can verify a selector resolved to the
  // RIGHT element (not just that something was visible).
  let last = "none";
  let gridChecked = false;
  const cells = [];
  for (const r of [1, 2, 3, 4]) {
    for (const c of ["A", "B", "C", "D"]) {
      cells.push(`${c}${r}`);
    }
  }
  function tap(label) {
    last = label;
  }
</script>

<div class="section">
  <h2>Selector Grid</h2>
  <div class="sel-grid">
    {#each cells as label}
      <button class="sel-cell" on:click={() => tap(label)}>{label}</button>
    {/each}
  </div>
  <button class="sel-wide" on:click={() => tap("WIDE")}>WIDE</button>
  <button class="sel-tall" on:click={() => tap("TALL")}>TALL</button>
  <div class="sel-dups">
    <button on:click={() => tap("DUP-1")}>DUP</button>
    <button on:click={() => tap("DUP-2")}>DUP</button>
  </div>
  <!-- disabled button (on_enabled = false) and a checkbox (on_checked) so the
       grid is a self-contained surface for state-filter selectors too. -->
  <button class="sel-cell" disabled>DIS</button>
  <label><input type="checkbox" aria-label="grid-check" bind:checked={gridChecked} /> Check</label>
  <!-- OCC: a wide button whose CENTRE is covered by an opaque overlay (a
       sibling painted on top). A naive centre tap hits the overlay; the
       occlusion-aware tap must route to a clear horizontal point and still
       fire OCC. -->
  <div class="occ-wrap">
    <button class="occ-btn" on:click={() => tap("OCC")}>OCC</button>
    <div class="occ-cover"></div>
  </div>
  <!-- Readout matched purely by text content; deliberately NO aria-label —
       an aria-label would become the accessible name and mask the text on iOS,
       breaking on_text matching. -->
  <span>tapped:{last}</span>
</div>
