<script>
let count = $state(0);
</script>
<div class="section">
  <h2>Counter</h2>
  <span>{count}</span>
  <button aria-label="Increment" onclick={() => count++}>+</button>
  <button aria-label="Decrement" onclick={() => count--}>-</button>
</div>
