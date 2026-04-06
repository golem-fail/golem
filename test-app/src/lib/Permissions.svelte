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
  <button onclick={requestCamera}>Request Camera</button>
  <button onclick={requestLocation}>Request Location</button>
</div>
