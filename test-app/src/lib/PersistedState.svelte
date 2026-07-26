<script>
// clear_data has no observable effect unless something survives a normal
// relaunch — the rest of the app is in-memory Svelte state that resets on
// launch anyway. localStorage persists across relaunch but IS wiped by
// clear_data (Android `pm clear`, iOS container uninstall), so it's the
// surface an e2e flow uses: set a value, relaunch (still there), clear_data,
// relaunch (back to the default). The readout renders the current value so a
// flow can assert both the persisted value and the post-clear reset.
const KEY = "golem_persisted_count";

function readStored() {
  const raw = localStorage.getItem(KEY);
  const n = raw === null ? 0 : parseInt(raw, 10);
  return Number.isNaN(n) ? 0 : n;
}

let count = $state(readStored());

function increment() {
  count += 1;
  localStorage.setItem(KEY, String(count));
}
</script>

<div class="section">
  <h2>Persisted State</h2>
  <div aria-label="persisted-readout">Persisted: {count}</div>
  <button aria-label="persist-increment" onclick={increment}>Increment persisted</button>
</div>
