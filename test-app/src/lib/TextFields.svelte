<script>
let email = $state("");
let password = $state("");
let search = $state("");
let multiline = $state("");
</script>
<div class="section">
  <h2>Text Fields</h2>
  <input type="email" aria-label="email-input" placeholder="Enter email" bind:value={email} />
  <input type="password" aria-label="password-input" placeholder="Password" bind:value={password} />
  <input type="search" aria-label="search-input" placeholder="Search" bind:value={search} />
  <textarea aria-label="multiline-input" placeholder="Multi-line text" bind:value={multiline}></textarea>
</div>
