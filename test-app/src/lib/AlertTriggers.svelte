<script>
import { ask, confirm, message } from "@tauri-apps/plugin-dialog";

let lastResult = $state("");

async function showAlert() {
  await message("This is a test alert", { title: "Alert", kind: "info" });
  lastResult = "Alert dismissed";
}

async function showConfirm() {
  const r = await confirm("Are you sure?", { title: "Confirm", kind: "warning" });
  lastResult = r ? "Confirm OK" : "Confirm Cancel";
}

async function showYesNo() {
  const r = await ask("Do you agree?", { title: "Question", kind: "info" });
  lastResult = r ? "Question Yes" : "Question No";
}
</script>
<div class="section">
  <h2>Alert Triggers</h2>
  <button onclick={showAlert}>Show Alert</button>
  <button onclick={showConfirm}>Show Confirm</button>
  <button onclick={showYesNo}>Show Yes/No</button>
  <div>{lastResult}</div>
</div>
