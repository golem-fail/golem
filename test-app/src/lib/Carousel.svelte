<div class="section">
  <h2>Carousel</h2>
  <div style="display:flex;overflow-x:auto;scroll-snap-type:x mandatory;gap:8px">
    {#each Array(10) as _, i}
      <div style="min-width:200px;scroll-snap-align:start;padding:20px;background:#eee;border-radius:8px">Card {i}</div>
    {/each}
  </div>
</div>
