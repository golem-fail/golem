<script>
import { onMount } from "svelte";
let theme = $state("Light");
let location = $state("0.0, 0.0");
let deeplink = $state("");
let notification = $state("");
let mediaCount = $state(0);
// Lifecycle: tracks the WebView's `document.visibilityState` and tags
// it with the most recent `visibilitychange` timestamp so we can tell
// at a glance whether the app has just bounced through background
// (e.g. an iOS deep-link confirmation prompt steals foreground while
// SpringBoard owns the dialog).
let lifecycle = $state("visible @ load");
let pageshowCount = $state(0);
let visibilityChangeCount = $state(0);

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
      const { onOpenUrl, getCurrent, register } = await import("@tauri-apps/plugin-deep-link");
      // Register first — on iOS this hooks the plugin into the
      // UIApplicationDelegate's openURL path so warm-start URLs are
      // forwarded to the JS listener. Without it, only cold-start URLs
      // (via getCurrent) are delivered.
      try { await register("golem-test"); } catch { /* already registered */ }
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

  // Lifecycle tracking. `visibilitychange` fires when the WebView is
  // backgrounded / foregrounded (iOS suspends timers while a system
  // dialog like the deep-link confirmation owns the screen).
  // `pageshow` fires when WKWebView re-presents the page (sometimes
  // with `event.persisted` true if it was bfcache-restored — rare
  // on Tauri but useful to surface).
  function onVisibility() {
    visibilityChangeCount += 1;
    lifecycle = `${document.visibilityState} (#${visibilityChangeCount})`;
  }
  function onPageShow(e) {
    pageshowCount += 1;
    lifecycle = `pageshow${e.persisted ? "*" : ""} (#${pageshowCount})`;
  }
  document.addEventListener("visibilitychange", onVisibility);
  window.addEventListener("pageshow", onPageShow);

  return () => {
    tMql.removeEventListener("change", updateTheme);
    if (unlistenDeepLink) unlistenDeepLink();
    document.removeEventListener("visibilitychange", onVisibility);
    window.removeEventListener("pageshow", onPageShow);
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
  <div class="row"><span>Lifecycle:</span> <span>{lifecycle}</span></div>
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
