<script>
import TextFields from "./lib/TextFields.svelte";
import Buttons from "./lib/Buttons.svelte";
import Toggles from "./lib/Toggles.svelte";
import Counter from "./lib/Counter.svelte";
import ScrollList from "./lib/ScrollList.svelte";
import Carousel from "./lib/Carousel.svelte";
import NestedLayout from "./lib/NestedLayout.svelte";
import AlertTriggers from "./lib/AlertTriggers.svelte";
import Permissions from "./lib/Permissions.svelte";
import DeviceState from "./lib/DeviceState.svelte";
import DelayedElement from "./lib/DelayedElement.svelte";
import DialogOverlay from "./lib/DialogOverlay.svelte";
import PositionTest from "./lib/PositionTest.svelte";
import GestureTarget from "./lib/GestureTarget.svelte";
import TouchVisualizer from "./lib/TouchVisualizer.svelte";
import Menu from "./lib/Menu.svelte";
</script>

<TouchVisualizer />
<Menu />
<main>
  <h1>GOLEM Test App</h1>
  <div id="section-counter"><Counter /></div>
  <div id="section-buttons"><Buttons /></div>
  <div id="section-text-fields"><TextFields /></div>
  <div id="section-toggles"><Toggles /></div>
  <div id="section-scroll-list"><ScrollList /></div>
  <div id="section-carousel"><Carousel /></div>
  <div id="section-nested-layout"><NestedLayout /></div>
  <div id="section-gesture-target"><GestureTarget /></div>
  <div id="section-alert-triggers"><AlertTriggers /></div>
  <div id="section-permissions"><Permissions /></div>
  <div id="section-device-state"><DeviceState /></div>
  <div id="section-delayed-element"><DelayedElement /></div>
  <div id="section-position-test"><PositionTest /></div>
  <div id="section-dialog-overlay"><DialogOverlay /></div>
</main>
