<script>
function requestCamera() {
  navigator.mediaDevices?.getUserMedia({ video: true }).catch(() => {});
}
function requestLocation() {
  navigator.geolocation?.getCurrentPosition(() => {}, () => {});
}
</script>
<div class="section">
  <h2>Permissions</h2>
  <button aria-label="camera-permission-button" onclick={requestCamera}>Request Camera</button>
  <button aria-label="location-permission-button" onclick={requestLocation}>Request Location</button>
</div>
