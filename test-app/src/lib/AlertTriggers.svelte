<script>
import { invoke } from "@tauri-apps/api/core";

async function showAlert() {
  await invoke("show_alert");
}

async function showConfirm() {
  const result = await invoke("show_confirm");
  console.log("Confirm result:", result);
}
</script>
<div class="section">
  <h2>Alert Triggers</h2>
  <button onclick={showAlert}>Show Alert</button>
  <button onclick={showConfirm}>Show Confirm</button>
</div>
