<!--
  A scrollable list whose items are each inside a per-row WRAPPER (the common
  <li><span> shape), unlike ScrollList where items are direct children. This
  reproduces the `within = { contains = ... }` ambiguity: the smallest box
  enclosing one "Row N" is its wrapper, not the scrollable container — so
  `contains = { text = "Row *", min_matches = 2 }` is needed to target the list.
-->
<div class="section">
  <h2>Wrapped List</h2>
  <ul class="wrapped-list" role="list" aria-label="wrapped-scroll-list">
    {#each Array(50) as _, i}
      <li class="row-wrapper" role="listitem">
        <span>Row {i}</span>
      </li>
    {/each}
  </ul>
</div>

<style>
.wrapped-list {
  max-height: 300px;
  overflow-y: auto;
  list-style: none;
  margin: 0;
  padding: 0;
  border: 1px solid #ccc;
}
/* Wrapper is full-width with padding, so its bounds are strictly larger than
   the inner text span — i.e. a genuine enclosing box, not coincident (which
   `contains` would otherwise discard). */
.row-wrapper {
  display: block;
  padding: 10px 16px;
  border-bottom: 1px solid #eee;
}
</style>
