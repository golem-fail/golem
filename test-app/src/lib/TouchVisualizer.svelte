<script>
import { onMount } from "svelte";

onMount(() => {
  // Tap: expanding ring at touch point
  document.addEventListener('touchstart', e => {
    for (const touch of e.touches) {
      const dot = document.createElement('div');
      dot.className = 'golem-tap-ring';
      dot.style.left = `${touch.clientX - 15}px`;
      dot.style.top = `${touch.clientY - 15}px`;
      document.body.appendChild(dot);
      setTimeout(() => dot.remove(), 500);
    }
  });

  // Swipe: fading trail dots along the path
  document.addEventListener('touchmove', e => {
    for (const touch of e.touches) {
      const dot = document.createElement('div');
      dot.className = 'golem-swipe-dot';
      dot.style.left = `${touch.clientX - 3}px`;
      dot.style.top = `${touch.clientY - 3}px`;
      document.body.appendChild(dot);
      setTimeout(() => dot.remove(), 300);
    }
  });
});
</script>

<style>
  :global(.golem-tap-ring) {
    position: fixed;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    border: 2px solid red;
    pointer-events: none;
    z-index: 99999;
    animation: tap-ring 0.5s ease-out forwards;
  }

  :global(.golem-swipe-dot) {
    position: fixed;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgba(255, 0, 0, 0.6);
    pointer-events: none;
    z-index: 99999;
    animation: swipe-fade 0.3s ease-out forwards;
  }

  @keyframes tap-ring {
    from { transform: scale(0.5); opacity: 1; }
    to { transform: scale(2); opacity: 0; }
  }

  @keyframes swipe-fade {
    from { opacity: 0.6; }
    to { opacity: 0; }
  }
</style>
