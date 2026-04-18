<script>
let scale = $state(1.0);
let zoomed = $state(false);
let fingerCount = $state(0);
let maxFingers = $state(0);

// 3x3 grid tracking
let grid = $state([0,0,0, 0,0,0, 0,0,0]);
let gridStr = $derived(
  `${grid[0]}${grid[1]}${grid[2]}-${grid[3]}${grid[4]}${grid[5]}-${grid[6]}${grid[7]}${grid[8]}`
);

// Coordinate tracking (relative to gesture area, 0-200 range)
let minX = $state(-1);
let minY = $state(-1);
let maxX = $state(-1);
let maxY = $state(-1);
let downX = $state(-1);
let downY = $state(-1);
let upX = $state(-1);
let upY = $state(-1);

let rotation = $state(0);    // cumulative degrees
let rotDir = $state("None"); // last rotation direction: "CW", "CCW", or "None"
let snappedRot = $derived(Math.round(rotation / 90) * 90);
let rotLabel = $derived(
  snappedRot === 0 && rotDir === "None" ? "0" :
  `${snappedRot} ${rotDir}`
);

let pointers = new Map();
let initialDistance = 0;
let prevAngle = 0;
let baseScale = 1.0;
let areaRect = null;

function getDistance(p1, p2) {
  const dx = p1.clientX - p2.clientX;
  const dy = p1.clientY - p2.clientY;
  return Math.sqrt(dx * dx + dy * dy);
}

function getAngle(p1, p2) {
  return Math.atan2(p2.clientY - p1.clientY, p2.clientX - p1.clientX) * 180 / Math.PI;
}

function relCoords(e) {
  if (!areaRect) return { x: 0, y: 0 };
  return {
    x: Math.round(e.clientX - areaRect.left),
    y: Math.round(e.clientY - areaRect.top),
  };
}

function markGrid(e) {
  if (!areaRect) return;
  const col = Math.floor(((e.clientX - areaRect.left) / areaRect.width) * 3);
  const row = Math.floor(((e.clientY - areaRect.top) / areaRect.height) * 3);
  if (col >= 0 && col < 3 && row >= 0 && row < 3) {
    grid[row * 3 + col] = 1;
  }
}

function updateMinMax(e) {
  const { x, y } = relCoords(e);
  if (minX < 0 || x < minX) minX = x;
  if (minY < 0 || y < minY) minY = y;
  if (x > maxX) maxX = x;
  if (y > maxY) maxY = y;
}

function onPointerDown(e) {
  e.preventDefault();
  areaRect = e.currentTarget.getBoundingClientRect();
  pointers.set(e.pointerId, e);
  fingerCount = pointers.size;
  if (fingerCount > maxFingers) maxFingers = fingerCount;
  markGrid(e);
  updateMinMax(e);

  const { x, y } = relCoords(e);
  downX = x;
  downY = y;

  if (pointers.size === 2) {
    const [p1, p2] = [...pointers.values()];
    initialDistance = getDistance(p1, p2);
    prevAngle = getAngle(p1, p2);
    baseScale = scale;
  }
}

function onPointerMove(e) {
  e.preventDefault();
  if (!pointers.has(e.pointerId)) return;
  pointers.set(e.pointerId, e);
  markGrid(e);
  updateMinMax(e);

  if (pointers.size === 2) {
    const [p1, p2] = [...pointers.values()];
    const currentDistance = getDistance(p1, p2);
    if (initialDistance > 0) {
      scale = Math.round(baseScale * (currentDistance / initialDistance) * 10) / 10;
      zoomed = Math.abs(scale - 1.0) > 0.2;
    }
    const currentAngle = getAngle(p1, p2);
    let delta = currentAngle - prevAngle;
    // Normalize frame-to-frame delta to -180..180
    if (delta > 180) delta -= 360;
    if (delta < -180) delta += 360;
    prevAngle = currentAngle;
    const newRotation = Math.round(rotation + delta);
    if (newRotation !== rotation) {
      rotDir = newRotation > rotation ? "CW" : "CCW";
      rotation = newRotation;
    }
  }
}

function onPointerUp(e) {
  const { x, y } = relCoords(e);
  upX = x;
  upY = y;
  pointers.delete(e.pointerId);
  fingerCount = pointers.size;
}

function reset() {
  scale = 1.0;
  zoomed = false;
  rotation = 0;
  rotDir = "None";
  fingerCount = 0;
  maxFingers = 0;
  grid = [0,0,0, 0,0,0, 0,0,0];
  minX = -1; minY = -1; maxX = -1; maxY = -1;
  downX = -1; downY = -1; upX = -1; upY = -1;
}
</script>

<div class="section" style="touch-action: none;">
  <h2>Gesture Target</h2>
  <div
    class="gesture-area"
    aria-label="gesture-area"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    style="touch-action: none;"
  >
    <div>{zoomed ? "Zoomed" : "Not zoomed"}</div>
    <div>Rot: {rotLabel}</div>
    <div>Deg: {rotation}</div>
    <div>Grid: {gridStr}</div>
    <div>Range: {minX},{minY} to {maxX},{maxY}</div>
    <div>Down: {downX},{downY} Up: {upX},{upY}</div>
    <div>Fingers: {fingerCount} Max: {maxFingers}</div>
  </div>
  <button onclick={reset}>Reset Gesture</button>
</div>

<style>
.gesture-area {
  width: 200px;
  height: 200px;
  background: #e8e8e8;
  border: 2px solid #999;
  border-radius: 8px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  user-select: none;
  margin: 8px auto;
  font-size: 12px;
  gap: 2px;
}
</style>
