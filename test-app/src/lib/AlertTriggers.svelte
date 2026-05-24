<script>
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { onMount } from "svelte";

let lastResult = $state("");

// Diagnostic log of every dialog-result that reached JS via either
// path (eval or event). Surfaced for golem to read on failure so
// we can tell which delivery path landed (or whether neither did).
// Initialized at module level so an eval that arrives before
// `onMount` still has somewhere to record.
if (typeof window !== "undefined" && !window.__golemDialogLog) {
  window.__golemDialogLog = [];
}

onMount(() => {
  // Belt + braces: subscribe to Tauri's `dialog-result` event AND
  // expose a window-level setter the Rust side can call directly
  // via `app.eval(...)`. Either path is sufficient on its own;
  // both run because Tauri's event listener has a registration
  // race on cold start (the listener's IPC handshake can lose to
  // a dialog that opens + dismisses very quickly), and the
  // window-level setter is unconditional — the eval lands the
  // moment the WebView is alive.
  window.__golemSetDialogResult = (payload) => {
    window.__golemDialogLog.push({ via: "eval", payload, t: Date.now() });
    lastResult = payload;
  };
  const unlisten = listen("dialog-result", (event) => {
    window.__golemDialogLog.push({ via: "event", payload: event.payload, t: Date.now() });
    lastResult = event.payload;
  });
  return () => { unlisten.then((u) => u()); };
});
</script>
<div class="section">
  <h2>Alert Triggers</h2>
  <button onclick={() => { lastResult = ""; invoke("show_alert"); }}>Show Alert</button>
  <button onclick={() => { lastResult = ""; invoke("show_confirm"); }}>Show Confirm</button>
  <button onclick={() => { lastResult = ""; invoke("show_yes_no"); }}>Show Yes/No</button>
  <div>{lastResult}</div>
</div>
