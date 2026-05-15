<script>
let email = $state("");
let password = $state("");
let search = $state("");
let multiline = $state("");
</script>
<div class="section">
  <h2>Text Fields</h2>
  <input type="email" placeholder="Enter email" bind:value={email} autocapitalize="off" autocorrect="off" />
  <input type="password" placeholder="Password" bind:value={password} autocapitalize="off" autocorrect="off" />
  <input type="search" placeholder="Search" bind:value={search} autocapitalize="off" autocorrect="off" />
  <textarea placeholder="Multi-line text" bind:value={multiline} autocapitalize="off" autocorrect="off"></textarea>
</div>
