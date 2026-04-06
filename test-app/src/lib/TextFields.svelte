<script>
let email = $state("");
let password = $state("");
let search = $state("");
let multiline = $state("");
</script>
<div class="section">
  <h2>Text Fields</h2>
  <input type="email" placeholder="Enter email" bind:value={email} />
  <input type="password" placeholder="Password" bind:value={password} />
  <input type="search" placeholder="Search" bind:value={search} />
  <textarea placeholder="Multi-line text" bind:value={multiline}></textarea>
</div>
