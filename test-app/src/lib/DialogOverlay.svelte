<script>
// Dialog A: uses CSS display toggle (always in DOM)
let showDialogA = $state(false);
let resultA = $state("");

// Dialog B: uses Svelte {#if} (DOM nodes added/removed)
let showDialogB = $state(false);
let resultB = $state("");

function openA() { showDialogA = true; resultA = ""; }
function acceptA() { resultA = "Accepted A"; showDialogA = false; }
function cancelA() { resultA = "Cancelled A"; showDialogA = false; }

function openB() { showDialogB = true; resultB = ""; }
function acceptB() { resultB = "Accepted B"; showDialogB = false; }
function cancelB() { resultB = "Cancelled B"; showDialogB = false; }
</script>

<div class="section">
  <h2>Dialog Overlay</h2>

  <div>
    <button onclick={openA}>Open Dialog A</button>
    <span>{resultA}</span>
  </div>
  <div>
    <button onclick={openB}>Open Dialog B</button>
    <span>{resultB}</span>
  </div>

  <!-- Dialog A: CSS display toggle -->
  <div class="dialog-backdrop" role="dialog" aria-label="Dialog A"
       style:display={showDialogA ? 'flex' : 'none'}>
    <div class="dialog-box">
      <p>Continue with A?</p>
      <div class="dialog-buttons">
        <button onclick={acceptA}>Accept A</button>
        <button onclick={cancelA}>Cancel A</button>
      </div>
    </div>
  </div>

  <!-- Dialog B: Svelte conditional rendering -->
  {#if showDialogB}
    <div class="dialog-backdrop" role="dialog" aria-label="Dialog B">
      <div class="dialog-box">
        <p>Continue with B?</p>
        <div class="dialog-buttons">
          <button onclick={acceptB}>Accept B</button>
          <button onclick={cancelB}>Cancel B</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }
  .dialog-box {
    background: white;
    border-radius: 12px;
    padding: 24px;
    min-width: 260px;
    text-align: center;
  }
  .dialog-buttons {
    display: flex;
    gap: 12px;
    justify-content: center;
    margin-top: 16px;
  }
</style>
