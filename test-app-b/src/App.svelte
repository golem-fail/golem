<script>
import { onMount } from "svelte";
let sharedData = $state("");
let statusText = $state("Ready");
let deeplinkUrl = $state("");

function parseDeepLink(url) {
  deeplinkUrl = url;
  try {
    const parsed = new URL(url);
    const data = parsed.searchParams.get("data");
    if (data) sharedData = data;
  } catch {
    const queryStart = url.indexOf("?");
    if (queryStart !== -1) {
      const params = new URLSearchParams(url.slice(queryStart));
      const data = params.get("data");
      if (data) sharedData = data;
    }
  }
}

function handleRefresh() {
  statusText = "Updated";
  setTimeout(() => { statusText = "Ready"; }, 2000);
}

onMount(async () => {
  try {
    const { onOpenUrl } = await import("@tauri-apps/plugin-deep-link");
    await onOpenUrl((urls) => {
      if (urls && urls.length > 0) parseDeepLink(urls[0]);
    });
  } catch (e) {
    console.warn("Deep link plugin not available:", e);
  }
});
</script>

<div class="header-section">
  <h1 aria-label="app-b-title">GOLEM Test B</h1>
</div>
<div class="content-section">
  <div aria-label="shared-data-display" role="status">{sharedData}</div>
  <button aria-label="refresh-button" onclick={handleRefresh}>Refresh</button>
  <span aria-label="status-label" role="status">{statusText}</span>
  <div aria-label="deeplink-display-b" role="status">{deeplinkUrl}</div>
</div>
