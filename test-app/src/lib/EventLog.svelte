<script>
import { eventLog } from "./eventLog.svelte.js";

// Pause logging while the panel is mounted. A tap fires pointerdown
// + several pointermove + pointerup; without the pause the entries
// list keeps shifting under the reader. Cleanup resumes capture so
// the next session sees fresh events.
$effect(() => {
  eventLog.paused = true;
  return () => { eventLog.paused = false; };
});
</script>

<section class="event-log" aria-label="event-log">
  <div class="event-log-header">
    <span class="event-log-title">Events ({eventLog.entries.length}) — paused</span>
    <button
      type="button"
      aria-label="clear-event-log"
      onclick={() => eventLog.clear()}
    >
      Clear
    </button>
  </div>
  {#if eventLog.entries.length === 0}
    <p class="event-log-empty">No events captured yet.</p>
  {:else}
    <ol class="event-log-entries">
      {#each eventLog.entries as e (e.ts + e.type + e.abs)}
        <li>
          {e.ts} {e.type} {e.abs} → {e.target} @ {e.rel}
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
.event-log {
  margin: 8px 0 0;
  padding: 6px;
  background: #fafafa;
  border: 1px solid #ddd;
  border-radius: 4px;
  /* Small font — pane exists mainly so `golem tree` can dump events as
     text. Humans can still read it, but density wins over legibility. */
  font-size: 9px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
.event-log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 3px;
}
.event-log-title {
  font-weight: 600;
}
.event-log-header button {
  font-size: 10px;
  padding: 2px 6px;
}
.event-log-empty {
  margin: 4px 0;
  color: #888;
  font-style: italic;
}
.event-log-entries {
  list-style: none;
  margin: 0;
  padding: 0;
  /* Large pane so a long flow's down/up pairs are all on-screen without
     the user having to scroll inside the log. Caps to viewport so it
     never exceeds the available space on small phones. */
  max-height: min(70vh, 600px);
  overflow-y: auto;
}
.event-log-entries li {
  padding: 2px 0;
  border-bottom: 1px dotted #eee;
  white-space: pre-wrap;
  word-break: break-all;
}
@media (prefers-color-scheme: dark) {
  .event-log {
    background: #1a1a1a;
    border-color: #444;
    color: #ddd;
  }
  .event-log-entries li {
    border-bottom-color: #333;
  }
  .event-log-empty {
    color: #777;
  }
}
</style>
