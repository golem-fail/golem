<script>
let delayedValue = $state("");
let timer = null;

function triggerDelay() {
  delayedValue = "";
  clearTimeout(timer);
  timer = setTimeout(() => {
    delayedValue = "Delayed Text";
    timer = setTimeout(() => {
      delayedValue = "";
    }, 3000);
  }, 3000);
}
</script>

<div class="section">
  <h2>Delayed Element</h2>
  <button onclick={triggerDelay}>Show Delayed</button>
  <input type="text" value={delayedValue} readonly />
</div>
