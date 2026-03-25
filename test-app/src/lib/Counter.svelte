<script>
let count = $state(0);
</script>
<div class="section">
  <h2>Counter</h2>
  <span aria-label="counter-display">{count}</span>
  <button aria-label="increment-button" onclick={() => count++}>+</button>
  <button aria-label="decrement-button" onclick={() => count--}>-</button>
</div>
