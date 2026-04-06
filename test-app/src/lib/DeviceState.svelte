<script>
import { onMount } from "svelte";
let orientation = $state("Portrait");
let theme = $state("Light");
let location = $state("0.0, 0.0");
let deeplink = $state("");
let notification = $state("");
let mediaCount = $state(0);

onMount(() => {
  function updateOrientation() {
    orientation = window.matchMedia("(orientation: landscape)").matches ? "Landscape" : "Portrait";
  }
  updateOrientation();
  const oMql = window.matchMedia("(orientation: landscape)");
  oMql.addEventListener("change", updateOrientation);

  function updateTheme() {
    theme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "Dark" : "Light";
  }
  updateTheme();
  const tMql = window.matchMedia("(prefers-color-scheme: dark)");
  tMql.addEventListener("change", updateTheme);

  window.__golemSetLocation = (lat, lon) => { location = `${lat}, ${lon}`; };
  window.__golemSetNotification = (payload) => { notification = String(payload); };
  window.__golemAddMedia = () => { mediaCount += 1; };
  window.__golemResetMediaCount = () => { mediaCount = 0; };

  return () => {
    oMql.removeEventListener("change", updateOrientation);
    tMql.removeEventListener("change", updateTheme);
  };
});
</script>
<div class="section">
  <h2>Device State</h2>
  <div><span>Orientation:</span> <span>{orientation}</span></div>
  <div><span>Theme:</span> <span>{theme}</span></div>
  <div><span>Location:</span> <span>{location}</span></div>
  <div><span>Deep Link:</span> <span>{deeplink}</span></div>
  <div><span>Notification:</span> <span>{notification}</span></div>
  <div><span>Media Count:</span> <span>{mediaCount}</span></div>
</div>
