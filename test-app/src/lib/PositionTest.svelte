<script>
// Test component for CSS positioning edge cases.
// Used to verify visibility confidence scoring handles:
// - position:fixed (should NOT be clipped by scrolled parent)
// - position:absolute overflowing parent (should NOT be clipped)
// - position:sticky (should work correctly — stays in container)
// - overflow:hidden (SHOULD be clipped — this is the bug we're fixing)
</script>

<div class="section">
  <h2>Position Test</h2>

  <!-- 1. Fixed position: button stays at bottom-right of screen -->
  <div class="fixed-container" style="position: relative; height: 60px; overflow: hidden; background: #f0f0f0; border: 1px solid #ccc;">
    <div style="position: fixed; bottom: 80px; right: 16px; z-index: 100;">
      <button class="fixed-btn">Fixed Button</button>
    </div>
    <span>Fixed parent (overflow:hidden)</span>
  </div>

  <!-- 2. Absolute position: badge overflows its parent -->
  <div style="position: relative; display: inline-block; margin: 8px 0;">
    <button>Notifications</button>
    <span class="badge" style="position: absolute; top: -8px; right: -8px; background: red; color: white; border-radius: 50%; width: 20px; height: 20px; display: flex; align-items: center; justify-content: center; font-size: 11px;">3</span>
  </div>

  <!-- 3. Overflow hidden scroll container with items -->
  <div class="clip-container" style="height: 80px; overflow: hidden; border: 1px solid #999; background: #fafafa;">
    <div>Visible Item A</div>
    <div>Visible Item B</div>
    <div>Clipped Item C</div>
    <div>Clipped Item D</div>
    <div>Clipped Item E</div>
  </div>

  <!-- 4. Sticky header in a scrollable area -->
  <div class="sticky-container" style="height: 100px; overflow: auto; border: 1px solid #999; background: #fafafa;">
    <div style="position: sticky; top: 0; background: #ddd; padding: 4px 8px; font-weight: bold;">Sticky Header</div>
    <div style="padding: 4px 8px;">Scroll Content 1</div>
    <div style="padding: 4px 8px;">Scroll Content 2</div>
    <div style="padding: 4px 8px;">Scroll Content 3</div>
    <div style="padding: 4px 8px;">Scroll Content 4</div>
    <div style="padding: 4px 8px;">Scroll Content 5</div>
    <div style="padding: 4px 8px;">Scroll Content 6</div>
  </div>
</div>

<style>
.fixed-btn {
  /* position:fixed ⇒ always on screen, so it lands in every flow's a11y audit.
     Dark green keeps white text ≥7:1 (AAA), clean at every level — the bad
     a11y fixtures live only in the A11yDemo section. */
  background: #1B5E20;
  color: white;
  border: none;
  padding: 8px 16px;
  border-radius: 20px;
  font-size: 12px;
  box-shadow: 0 2px 8px rgba(0,0,0,0.3);
}
</style>
