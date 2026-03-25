<div class="section">
  <h2>Scroll List</h2>
  <div role="list" aria-label="scroll-list" style="max-height:300px;overflow-y:auto">
    {#each Array(50) as _, i}
      <div role="listitem" aria-label="list-item-{i}">Item {i}</div>
    {/each}
  </div>
</div>
