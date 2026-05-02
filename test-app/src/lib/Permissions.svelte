<script>
// Tests only need the button taps to *register* — they don't need the
// platform API to actually do anything. iOS sim has no camera/location
// subsystem to satisfy, so a real `getUserMedia` / `getCurrentPosition`
// call is just an extra footgun.
let cameraTaps = $state(0);
let locationTaps = $state(0);
</script>
<div class="section">
  <h2>Permissions</h2>
  <button aria-label="camera-permission-button" onclick={() => cameraTaps++}>Request Camera</button>
  <span aria-label="camera-tap-count">{cameraTaps}</span>
  <button aria-label="location-permission-button" onclick={() => locationTaps++}>Request Location</button>
  <span aria-label="location-tap-count">{locationTaps}</span>
</div>
