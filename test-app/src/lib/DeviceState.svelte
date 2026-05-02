<script>
import { onMount } from "svelte";
let theme = $state("Light");
let location = $state("0.0, 0.0");
let deeplink = $state("");
let notification = $state("");
let mediaCount = $state(0);

onMount(() => {
  function updateTheme() {
    theme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "Dark" : "Light";
  }
  updateTheme();
  const tMql = window.matchMedia("(prefers-color-scheme: dark)");
  tMql.addEventListener("change", updateTheme);

  // Deep link: dynamically import so a non-Tauri (e.g. `vite preview`)
  // build doesn't fail on the import. Subscribe both for cold-start
  // (`getCurrent`) and warm-start (`onOpenUrl`) deliveries. iOS
  // currently fails to deliver — see "Deep-link delivery on iOS"
  // in docs/roadmap.md.
  let unlistenDeepLink;
  (async () => {
    try {
      const { onOpenUrl, getCurrent } = await import("@tauri-apps/plugin-deep-link");
      try {
        const urls = await getCurrent();
        if (urls && urls.length > 0) deeplink = urls[urls.length - 1];
      } catch { /* not mobile / no current url */ }
      try {
        unlistenDeepLink = await onOpenUrl((urls) => {
          if (urls && urls.length > 0) deeplink = urls[urls.length - 1];
        });
      } catch { /* listener install failed */ }
    } catch { /* plugin not available in this build */ }
  })();

  window.__golemSetLocation = (lat, lon) => { location = `${lat}, ${lon}`; };
  window.__golemSetNotification = (payload) => { notification = String(payload); };
  window.__golemAddMedia = () => { mediaCount += 1; };
  window.__golemResetMediaCount = () => { mediaCount = 0; };

  return () => {
    tMql.removeEventListener("change", updateTheme);
    if (unlistenDeepLink) unlistenDeepLink();
  };
});
</script>
<div class="section">
  <h2>Device State</h2>
  <div class="row"><span>Theme:</span> <span>{theme}</span></div>
  <div class="row"><span>Location:</span> <span>{location}</span></div>
  <div class="row"><span>Deep Link:</span> <span>{deeplink}</span></div>
  <div class="row"><span>Notification:</span> <span>{notification}</span></div>
  <div class="row"><span>Media Count:</span> <span>{mediaCount}</span></div>
</div>

<style>
/* iOS WebKit Inspector reports 0x0 bounds for inline <span>s inside
   plain block <div>s — accessibility tools (golem included) then can't
   place them in the viewport. Flexing the row + giving the spans a
   minimum width forces the layout engine to resolve real bounds. */
.row {
  display: flex;
  gap: 8px;
  align-items: baseline;
  min-height: 20px;
}
.row span {
  display: inline-block;
  min-width: 20px;
}
</style>
