<script>
import EventLog from "./EventLog.svelte";

let open = $state(false);
let logsOpen = $state(false);

const links = [
  { id: "counter",         label: "Counter" },
  { id: "buttons",         label: "Buttons" },
  { id: "text-fields",     label: "Text Fields" },
  { id: "toggles",         label: "Toggles" },
  { id: "scroll-list",     label: "Scroll List" },
  { id: "carousel",        label: "Carousel" },
  { id: "nested-layout",   label: "Nested Layout" },
  { id: "gesture-target",  label: "Gesture Target" },
  { id: "alert-triggers",  label: "Alert Triggers" },
  { id: "permissions",     label: "Permissions" },
  { id: "device-state",    label: "Device State" },
  { id: "delayed-element", label: "Delayed Element" },
  { id: "position-test",   label: "Position Test" },
  { id: "dialog-overlay",  label: "Dialog Overlay" },
];

function gotoSection(id) {
  // Close the menu BEFORE scrolling. With the menu open, the scroll
  // target is computed against the expanded layout, then closing the
  // menu reflows the page and iOS WKWebView's scroll anchoring leaves
  // us ~100px overshot. Defer the scroll one frame so layout settles.
  open = false;
  requestAnimationFrame(() => {
    const el = document.getElementById(`section-${id}`);
    if (!el) return;
    const cs = window.getComputedStyle(el);
    const margin = parseInt(cs.scrollMarginTop, 10) || 0;
    // Manual `scrollTo` instead of `scrollIntoView({block:"start"})` —
    // iPad WKWebView in viewport-fit=cover mode mishandles
    // `scroll-margin-top` and overshoots; computing the target
    // explicitly is deterministic across iOS / Android / desktop.
    window.scrollTo({ top: el.offsetTop - margin, behavior: "instant" });
  });
}
</script>

<nav class="menu" aria-label="section-navigation">
  <div class="menu-bar">
    <button
      type="button"
      class="menu-toggle"
      aria-label="menu-toggle"
      onclick={() => (open = !open)}
    >
      Menu
    </button>
    <button
      type="button"
      class="menu-toggle"
      aria-label="logs-toggle"
      onclick={() => (logsOpen = !logsOpen)}
    >
      Logs
    </button>
  </div>
  {#if open}
    <ul class="menu-links">
      {#each links as link (link.id)}
        <li>
          <button
            type="button"
            aria-label="goto-{link.id}"
            onclick={() => gotoSection(link.id)}
          >
            {link.label}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if logsOpen}
    <EventLog />
  {/if}
</nav>

<style>
.menu {
  position: sticky;
  top: 0;
  z-index: 10;
  background: var(--menu-bg, #fff);
  border-bottom: 1px solid #ccc;
  /* Padding-top includes the device's safe-area inset so the menu
     isn't hidden under the iOS notch / Android status bar. */
  padding: max(8px, env(safe-area-inset-top, 8px)) 8px 4px;
}
/* Sections we jump to via the menu need a scroll-margin-top equal to
   the (closed) menu height + the device's safe-area inset so that
   `scrollIntoView` doesn't park them under the sticky menu bar. */
:global([id^="section-"]) {
  scroll-margin-top: calc(60px + env(safe-area-inset-top, 0px));
}
.menu-bar {
  display: flex;
  gap: 6px;
}
.menu-toggle {
  font-size: 14px;
  padding: 6px 12px;
}
.menu-links {
  list-style: none;
  margin: 8px 0 4px;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.menu-links li {
  flex: 0 0 auto;
}
.menu-links button {
  font-size: 12px;
  padding: 4px 8px;
  background: #f4f4f4;
  border: 1px solid #ddd;
  border-radius: 4px;
  white-space: nowrap;
}
@media (prefers-color-scheme: dark) {
  .menu {
    background: #222;
    border-bottom-color: #444;
  }
  .menu-links button {
    background: #333;
    border-color: #555;
    color: #fff;
  }
}
</style>
