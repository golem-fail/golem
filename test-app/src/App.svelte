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
import TouchVisualizer from "./lib/TouchVisualizer.svelte";
</script>

<TouchVisualizer />
<main>
  <h1>GOLEM Test App</h1>
  <Counter />
  <Buttons />
  <TextFields />
  <Toggles />
  <ScrollList />
  <Carousel />
  <NestedLayout />
  <AlertTriggers />
  <Permissions />
  <DeviceState />
  <DelayedElement />
  <PositionTest />
  <DialogOverlay />
</main>
