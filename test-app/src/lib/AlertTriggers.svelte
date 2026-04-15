<script>
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { onMount } from "svelte";

let lastResult = $state("");

onMount(() => {
  listen("dialog-result", (event) => {
    lastResult = event.payload;
  });
});
</script>
<div class="section">
  <h2>Alert Triggers</h2>
  <button onclick={() => { lastResult = ""; invoke("show_alert"); }}>Show Alert</button>
  <button onclick={() => { lastResult = ""; invoke("show_confirm"); }}>Show Confirm</button>
  <button onclick={() => { lastResult = ""; invoke("show_yes_no"); }}>Show Yes/No</button>
  <div>{lastResult}</div>
</div>
