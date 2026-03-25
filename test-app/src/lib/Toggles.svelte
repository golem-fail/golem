<script>
let darkMode = $state(false);
let notifications = $state(false);
let agreed = $state(false);
</script>
<div class="section">
  <h2>Toggles</h2>
  <label>
    <span>Dark Mode</span>
    <button role="switch" aria-label="dark-mode-toggle" aria-checked={darkMode} onclick={() => darkMode = !darkMode}>{darkMode ? "On" : "Off"}</button>
  </label>
  <label>
    <span>Notifications</span>
    <button role="switch" aria-label="notifications-toggle" aria-checked={notifications} onclick={() => notifications = !notifications}>{notifications ? "On" : "Off"}</button>
  </label>
  <label>
    <input type="checkbox" aria-label="agree-checkbox" bind:checked={agreed} />
    <span>I agree</span>
  </label>
</div>
