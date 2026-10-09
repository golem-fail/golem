<!-- Generated from golem-cli (mcp.rs, help.rs) and docs/src. Run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-cli mcp_context`. -->
# MCP context

What an MCP client receives from `golem mcp`, on one page: the instructions, the tool list, and each answer of `help`. For golem developers; [MCP server](mcp.md) is the user guide.

| Text | Characters |
|------|-----------:|
| Instructions | 347 |
| Tool list (30 tools, as `tools/list` JSON) | 11933 |
| Both, kept in context by most clients | 12280 |

## Instructions

```text
golem (MCP server): Mobile e2e testing on iOS and Android simulators and emulators. Use it when the user wants to write, edit or run an e2e test flow (.test.toml) for a mobile app, or to reproduce and debug a mobile app bug: drive the screen, read the UI tree, read crash logs. Start with session_open; help() explains the steps and the flow file.
```

## Tools

### `act`

Run one step on the device: a one-line TOML inline table, such as { action = "tap", on_text = "Sign in" }; help("act") lists the actions. Prefer on_text, the text a user reads; use on_accessibility_label only to test that label. A step that passes goes into the flow draft at the cursor, as written (a ${var} stays a reference); a failed step does not.

| Parameter | Type | Description |
|-----------|------|-------------|
| `comment` | string | Goes above the step in the draft. |
| `format` | string | "toon" (compact text, default) or "json". |
| `step` (required) | string |  |
| `tree` | boolean | Also return the visible tree after the step. |

### `app_logs`

The app's device log (Android logcat, iOS simulator; not physical iOS): crash lines first, then the newest. Answers while another operation runs.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` | string | An app name or bundle id. Default: the session's app. |
| `filter` | string | Only lines with this text, in any case. |
| `limit` | integer | Lines besides crash lines. Default 200, the newest. |
| `since` | integer | Seconds back. Default: since the session opened. |

### `apps_set`

Add or replace a [[flow.apps]] entry in the draft. The session does not change.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` (required) | object | { "name": "app", "bundle": "com.acme", "devices": [{ "os": "ios:latest" }] }; also permissions, install_script. The same name replaces those fields. |

### `block_begin`

Move the cursor to the end of a block; a new name adds the block at the end.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |
| `next` | string | Its next block. |

### `block_delete`

Remove a block and its steps. Refused while a next, goto or start names it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |

### `block_link`

Set a block's next, and add branches. After its steps a block takes the first branch that holds. A block with branches ignores next: with no match it goes to the next block in the file. Without branches: next, else the next block.

| Parameter | Type | Description |
|-----------|------|-------------|
| `block` (required) | string |  |
| `branches` | array | [{ "if_visible": "Error", "goto": "retry" }]; or if_not_visible, or if_var with equals, matches or gte. |
| `next` | string |  |

### `block_rename`

Rename a block, and each next, goto and start that names it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |
| `to` (required) | string |  |

### `cancel`

Stop the running operation. No teardown runs.

### `comment_add`

Add a comment line at the cursor.

| Parameter | Type | Description |
|-----------|------|-------------|
| `text` (required) | string |  |

### `data_add`

Add a [[data]] row. A block with for_each = "data" runs once per row; its steps read ${_each.field}.

| Parameter | Type | Description |
|-----------|------|-------------|
| `row` (required) | object | { "email": "a@b.test", "name": "Ada" } |

### `devices`

Devices in any state: platform, id, name, OS, state, live companion port.

| Parameter | Type | Description |
|-----------|------|-------------|
| `os` | string | "ios", "android", "ios:26", "ios:26+" or "ios:latest". |

### `draft_run`

Run the draft on the device: no setup, no teardown, the app as it is. Steps that pass become ✓, also from ? or ~. Stops before stop_at, at a failed step, or at the end; the cursor goes there.

| Parameter | Type | Description |
|-----------|------|-------------|
| `restart` | boolean | true: from the start. Default false: from the cursor. |
| `stop_at` | string | Stop before this step: block or block:step. |

### `draft_show`

The flow draft as .test.toml text: the opened flow, or a new flow with block main.

### `draft_steps`

The draft's steps near the cursor, or one block's, each as block:step (from 1), status, step and comment. Status: ✓ passed here, · not run here, ? unverified (# unverified in the file), ~ stale. A change (a new, edited, moved or deleted step) makes the next step ? and later steps ~. Block headers show next and branches.

| Parameter | Type | Description |
|-----------|------|-------------|
| `around` | string | "cursor" (default) or block:step. |
| `block` | string | List this block instead. |
| `context` | integer | Steps on each side. Default 5. |
| `limit` | integer |  |

### `export_flow`

Validate the draft as golem run does; if valid, write it. Reports the status counts and the unverified steps.

| Parameter | Type | Description |
|-----------|------|-------------|
| `overwrite` | boolean | Needed to replace a file this session did not open or export. |
| `path` (required) | string | Relative to the project, or absolute. |

### `flow_set`

Set [flow] fields of the draft.

| Parameter | Type | Description |
|-----------|------|-------------|
| `explicit_only` | boolean | true: golem run without a path skips this flow. |
| `name` | string |  |
| `seed` | integer | The seed for fake: generators. |
| `start` | string | The first block. |
| `tags` | array |  |
| `vars` | object | Merged into [flow] vars. |

### `help`

The docs, one piece at a time: help() lists the topics, help(topic) a topic's items, help(topic, item) one item, such as help("act", "tap").

| Parameter | Type | Description |
|-----------|------|-------------|
| `item` | string | An action, a group, a section or a code. |
| `topic` | string | act, selectors, flow, fake or codes. |

### `mixins_list`

The project's mixins and the vars each uses. Run one with act: { action = "load_mixin", mixin = "name", vars = { … } }.

### `probe`

What a selector matches, without acting: each visible match, which one act picks, each anchor; on a miss, which clause removed the candidates. Never fails.

| Parameter | Type | Description |
|-----------|------|-------------|
| `format` | string | "toon" (default) or "json". |
| `selector` (required) | string | { on_text = "Sign in" }; a whole step works too. |
| `timeout_ms` | integer | Poll this long while nothing matches. Default 0. |

### `record_only`

Record a step at the cursor without running it, for a path the session does not take. It is ? (unverified), and so is the step after it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `comment` | string |  |
| `step` (required) | string |  |

### `screenshot`

The screen as a PNG.

### `session_close`

Close the session; release the device.

| Parameter | Type | Description |
|-----------|------|-------------|
| `teardown` | boolean | Run the flow's [[teardown]]. Default true. |

### `session_open`

Open a session on one device and app. A running device that fits wins; else golem boots one. The session ends at session_close, when this server stops, or after idle_timeout_s idle.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` | string | The app's name in golem.toml [[apps]]. |
| `boot` | boolean | Boot a device when no running one fits. Default true. |
| `break_on_failure` | boolean | With flow: open at a failed step instead of ending. |
| `bundle` | string | The app's bundle id. |
| `device` | string | A UDID, a serial, a name, or part of one. |
| `flow` | string | Run this .test.toml first, as golem run does; the session opens where it stops. Its [[teardown]] runs at session_close. |
| `idle_timeout_s` | integer | End the session after this many idle seconds. Default 1800. |
| `os` | string | "ios", "android", "ios:26", "ios:26+" or "ios:latest". Default: any. |
| `project` | string | The directory with golem.toml. Default: the server's project. |
| `run` | boolean | With flow: false does the setup but runs no steps. |
| `stop_at` | string | With flow: stop before this step: block or block:step. |
| `teardown` | boolean | With flow: false never runs its [[teardown]]. |
| `type` | string | "phone" or "tablet". |
| `vars` | object | With flow: variables, as golem run --var. |

### `status`

Idle or busy, without waiting: the running operation and its phase, or the last result. While busy, only status, wait, cancel, app_logs and session_close answer.

### `step_delete`

Remove a draft step and its comment. The next step becomes ?, and later steps ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `at` (required) | string | block:step, from 1. |

### `step_edit`

Change a draft step without running it. Only a new comment or a larger timeout keeps its status; any other change makes it and the next step ?, and later steps ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `at` (required) | string | block:step, from 1. |
| `comment` | string | "" removes it. |
| `step` | string | The whole new step. |

### `step_move`

Move a draft step. Where it was counts as a delete; where it lands it is ?.

| Parameter | Type | Description |
|-----------|------|-------------|
| `from` (required) | string | block:step, from 1. |
| `to` (required) | string | Its block:step after the move, counted without the moved step; one past a block's last step appends. |

### `teardown_add`

Add a step to the draft's [[teardown]]. It does not run.

| Parameter | Type | Description |
|-----------|------|-------------|
| `comment` | string |  |
| `step` (required) | string |  |

### `tree`

The screen: one line per element you can target. full adds off-screen elements, as a hint only. Target an element by selector keys, not by its index.

| Parameter | Type | Description |
|-----------|------|-------------|
| `format` | string | "toon" (default) or "json". |
| `full` | boolean | Add off-screen elements: a hint, never proof. |

### `wait`

Wait for the operation that answered pending, and return its result (or the last result). It can answer pending again.

| Parameter | Type | Description |
|-----------|------|-------------|
| `timeout_s` | integer | Default: the server's soft timeout. |

## Help

Each call that `help` answers, linked to its text: the section of the docs page, or the `.llm.md` file that replaces it for the LLM. The size is the answer's, in characters.

```text
help(topic) lists a topic's items; help(topic, item) gives one item.

Topics:
- act: the step notation and every action, in groups
- selectors: how a step finds its element: text, label, anchors, traits
- flow: the .test.toml file: blocks, branches, steps, vars, data, teardown, devices
- fake: fake data such as ${fake:email}, person, address, credit_card
- codes: failure codes such as EF404: what each means and its fix
```

### act

- `help("act")` · [Actions (LLM text)](src/actions-reference/00-intro.llm.md) · 2970
- `help("act", "interaction")` · [Interaction](actions-reference.md#interaction) · 449
  - `help("act", "tap")` · [`tap` — Tap an element](actions-reference.md#tap--tap-an-element) · 439
  - `help("act", "double_tap")` · [`double_tap` — Double-tap an element](actions-reference.md#double_tap--double-tap-an-element) · 177
  - `help("act", "type")` · [`type` — Type text into an element](actions-reference.md#type--type-text-into-an-element) · 491
  - `help("act", "backspace")` · [`backspace` — Delete characters](actions-reference.md#backspace--delete-characters) · 488
  - `help("act", "clear_text")` · [`clear_text` — Empty a field](actions-reference.md#clear_text--empty-a-field) · 806
  - `help("act", "long_press")` · [`long_press` — Long press an element](actions-reference.md#long_press--long-press-an-element) · 271
  - `help("act", "swipe")` · [`swipe` — Swipe gesture](actions-reference.md#swipe--swipe-gesture) · 1481
  - `help("act", "scroll")` · [`scroll` — Scroll until element found](actions-reference.md#scroll--scroll-until-element-found) · 781
  - `help("act", "pinch")` · [`pinch` — Pinch zoom gesture](actions-reference.md#pinch--pinch-zoom-gesture) · 748
  - `help("act", "gesture")` · [`gesture` — Multi-touch gesture](actions-reference.md#gesture--multi-touch-gesture) · 810
  - `help("act", "rotate")` · [`rotate` — Rotate gesture](actions-reference.md#rotate--rotate-gesture) · 870
  - `help("act", "hide_keyboard")` · [`hide_keyboard` — Dismiss keyboard](actions-reference.md#hide_keyboard--dismiss-keyboard) · 465
- `help("act", "assertions")` · [Assertions](actions-reference.md#assertions) · 282
  - `help("act", "assert_visible")` · [`assert_visible` — Wait for / assert element exists](actions-reference.md#assert_visible--wait-for--assert-element-exists) · 706
  - `help("act", "assert_not_visible")` · [`assert_not_visible` — Wait for / assert element absent](actions-reference.md#assert_not_visible--wait-for--assert-element-absent) · 435
  - `help("act", "assert_alert")` · [`assert_alert` — Assert alert is displayed](actions-reference.md#assert_alert--assert-alert-is-displayed) · 230
- `help("act", "reading")` · [Reading](actions-reference.md#reading) · 106
  - `help("act", "read")` · [`read` — Read element text](actions-reference.md#read--read-element-text) · 344
- `help("act", "app-lifecycle")` · [App Lifecycle](actions-reference.md#app-lifecycle) · 228
  - `help("act", "launch")` · [`launch` — Launch or foreground an app](actions-reference.md#launch--launch-or-foreground-an-app) · 904
  - `help("act", "app-permissions")` · [App permissions (LLM text)](src/actions-reference/40-app-lifecycle/20-app-permissions.llm.md) · 1620
  - `help("act", "stop")` · [`stop` — Terminate an app](actions-reference.md#stop--terminate-an-app) · 143
  - `help("act", "clear_data")` · [`clear_data` — Clear app data](actions-reference.md#clear_data--clear-app-data) · 654
- `help("act", "device-controls")` · [Device Controls](actions-reference.md#device-controls) · 209
  - `help("act", "set_dark_mode")` · [`set_dark_mode` — Set dark mode](actions-reference.md#set_dark_mode--set-dark-mode) · 343
  - `help("act", "set_location")` · [`set_location` — Set GPS coordinates](actions-reference.md#set_location--set-gps-coordinates) · 310
  - `help("act", "press")` · [`press` — Press hardware button](actions-reference.md#press--press-hardware-button) · 495
- `help("act", "capture")` · [Capture](actions-reference.md#capture) · 215
  - `help("act", "screenshot")` · [`screenshot` — Take screenshot](actions-reference.md#screenshot--take-screenshot) · 298
  - `help("act", "recording")` · [Screen recording — per-block via `record = true`](actions-reference.md#screen-recording--per-block-via-record--true) · 547
  - `help("act", "add_media")` · [`add_media` — Push media to device](actions-reference.md#add_media--push-media-to-device) · 537
- `help("act", "alerts")` · [Alerts](actions-reference.md#alerts) · 152
  - `help("act", "accept_alert")` · [`accept_alert` — Accept dialog](actions-reference.md#accept_alert--accept-dialog) · 415
  - `help("act", "dismiss_alert")` · [`dismiss_alert` — Dismiss dialog](actions-reference.md#dismiss_alert--dismiss-dialog) · 364
- `help("act", "external")` · [External](actions-reference.md#external) · 460
  - `help("act", "open_link")` · [`open_link` — Open URL or deep link](actions-reference.md#open_link--open-url-or-deep-link) · 203
  - `help("act", "push_notification")` · [`push_notification` — Deliver a push to the app under test (LLM text)](src/actions-reference/80-external/20-push_notification.llm.md) · 950
  - `help("act", "bash")` · [`bash` — Run shell command](actions-reference.md#bash--run-shell-command) · 312
  - `help("act", "run")` · [`run` — Run project script](actions-reference.md#run--run-project-script) · 637
  - `help("act", "create_inbox")` · [`create_inbox` — Provision a disposable email inbox](actions-reference.md#create_inbox--provision-a-disposable-email-inbox) · 1341
  - `help("act", "await_email")` · [`await_email` — Poll IMAP inbox](actions-reference.md#await_email--poll-imap-inbox) · 1324
  - `help("act", "load_fixture")` · [`load_fixture` — Load fixture data](actions-reference.md#load_fixture--load-fixture-data) · 545
  - `help("act", "load_mixin")` · [`load_mixin` — Inline a reusable step sequence](actions-reference.md#load_mixin--inline-a-reusable-step-sequence) · 711
  - `help("act", "http")` · [`get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests](actions-reference.md#get_http-post_http-put_http-patch_http-delete_http--http-requests) · 670
- `help("act", "browser")` · [Browser (LLM text)](src/actions-reference/90-browser/00-intro.llm.md) · 2233
  - `help("act", "browse_navigate")` · [`browse_navigate` — Load a URL (LLM text)](src/actions-reference/90-browser/10-browse_navigate.llm.md) · 811
  - `help("act", "browse_tap")` · [`browse_tap` — Click an element](actions-reference.md#browse_tap--click-an-element) · 211
  - `help("act", "browse_type")` · [`browse_type` — Type into a field](actions-reference.md#browse_type--type-into-a-field) · 404
  - `help("act", "browse_read")` · [`browse_read` — Read text or an attribute into a variable](actions-reference.md#browse_read--read-text-or-an-attribute-into-a-variable) · 597
  - `help("act", "browse_screenshot")` · [`browse_screenshot` — Capture the tab](actions-reference.md#browse_screenshot--capture-the-tab) · 316
  - `help("act", "browse_assert_exists")` · [`browse_assert_exists` — The element is in the DOM](actions-reference.md#browse_assert_exists--the-element-is-in-the-dom) · 241
  - `help("act", "browse_assert_not_exists")` · [`browse_assert_not_exists` — The element is not in the DOM](actions-reference.md#browse_assert_not_exists--the-element-is-not-in-the-dom) · 291
  - `help("act", "browse_assert_text")` · [`browse_assert_text` — The element says what you expect](actions-reference.md#browse_assert_text--the-element-says-what-you-expect) · 801
  - `help("act", "browse_wait_exists")` · [`browse_wait_exists` — Wait for an element to appear](actions-reference.md#browse_wait_exists--wait-for-an-element-to-appear) · 333
  - `help("act", "browse_wait_not_exists")` · [`browse_wait_not_exists` — Wait for an element to disappear](actions-reference.md#browse_wait_not_exists--wait-for-an-element-to-disappear) · 319
  - `help("act", "browse_scroll_by")` · [`browse_scroll_by` — Scroll by a distance](actions-reference.md#browse_scroll_by--scroll-by-a-distance) · 592
  - `help("act", "browse_scroll_to")` · [`browse_scroll_to` — Bring an element into view](actions-reference.md#browse_scroll_to--bring-an-element-into-view) · 208
  - `help("act", "browse_select")` · [`browse_select` — Choose an option in a `<select>`](actions-reference.md#browse_select--choose-an-option-in-a-select) · 542
  - `help("act", "browse_execute_js")` · [`browse_execute_js` — Run JavaScript in the page (LLM text)](src/actions-reference/90-browser/140-browse_execute_js.llm.md) · 843
  - `help("act", "browse_mcp_list_tools")` · [`browse_mcp_list_tools` — List the page's WebMCP tools](actions-reference.md#browse_mcp_list_tools--list-the-pages-webmcp-tools) · 423
  - `help("act", "browse_mcp_call")` · [`browse_mcp_call` — Call a WebMCP tool](actions-reference.md#browse_mcp_call--call-a-webmcp-tool) · 870
  - `help("act", "browse_cookies")` · [`browse_set_cookie` / `browse_get_cookie` — Cookies](actions-reference.md#browse_set_cookie--browse_get_cookie--cookies) · 738
  - `help("act", "browse_local_storage")` · [`browse_set_local_storage` / `browse_get_local_storage` — Local storage](actions-reference.md#browse_set_local_storage--browse_get_local_storage--local-storage) · 657
  - `help("act", "browse_session_storage")` · [`browse_set_session_storage` / `browse_get_session_storage` — Session storage](actions-reference.md#browse_set_session_storage--browse_get_session_storage--session-storage) · 304
  - `help("act", "browse_close")` · [`browse_close` — Close a tab early](actions-reference.md#browse_close--close-a-tab-early) · 208
- `help("act", "flow-control")` · [Flow Control](actions-reference.md#flow-control) · 104
  - `help("act", "fail")` · [`fail` — Fail the flow immediately](actions-reference.md#fail--fail-the-flow-immediately) · 459

### selectors

- `help("selectors")` · [Selectors](selectors.md#selectors) · 899
- `help("selectors", "two-syntaxes")` · [Two syntaxes](selectors.md#two-syntaxes) · 365
- `help("selectors", "core")` · [Core selectors](selectors.md#core-selectors) · 1120
  - `help("selectors", "prefer-text")` · [Prefer visible `text`; use `accessibility_label` sparingly](selectors.md#prefer-visible-text-use-accessibility_label-sparingly) · 668
- `help("selectors", "state-filters")` · [State filters](selectors.md#state-filters) · 296
- `help("selectors", "traits")` · [Traits](selectors.md#traits) · 583
- `help("selectors", "relational")` · [Relational (positional) selectors](selectors.md#relational-positional-selectors) · 1167
- `help("selectors", "containment")` · [Geometric containment: `contains` / `inside`](selectors.md#geometric-containment-contains--inside) · 898
  - `help("selectors", "min_matches")` · [`min_matches` — the container of *repeated* items](selectors.md#min_matches--the-container-of-repeated-items) · 1154
- `help("selectors", "chaining")` · [Nesting and chaining (LLM text)](src/selectors/70-chaining.llm.md) · 648
- `help("selectors", "occlusion")` · [Occlusion-aware tapping](selectors.md#occlusion-aware-tapping) · 1051
- `help("selectors", "within")` · [`within` (scoping a scroll)](selectors.md#within-scoping-a-scroll) · 710
- `help("selectors", "canvas")` · [Canvas-rendered UI (Compose, Compose Multiplatform, Flutter) (LLM text)](src/selectors/100-canvas/00-intro.llm.md) · 2155
  - `help("selectors", "version-floors")` · [Version floors](selectors.md#version-floors) · 201
  - `help("selectors", "annotations")` · [Which annotation reaches golem](selectors.md#which-annotation-reaches-golem) · 1264
  - `help("selectors", "element-types")` · [Element types](selectors.md#element-types) · 1086
  - `help("selectors", "bounds")` · [Bounds and visibility](selectors.md#bounds-and-visibility) · 1010
  - `help("selectors", "merged-semantics")` · [Merged semantics](selectors.md#merged-semantics) · 1250
  - `help("selectors", "flutter")` · [Flutter](selectors.md#flutter) · 164

### flow

- `help("flow")` · [Test Structure](test-structure.md#test-structure) · 742
- `help("flow", "flow")` · [Flow](test-structure.md#flow) · 1272
  - `help("flow", "launch-permissions")` · [Launch-time permissions](test-structure.md#launch-time-permissions) · 829
  - `help("flow", "flow/options")` · [Flow Options](test-structure.md#flow-options) · 1992
  - `help("flow", "accessibility-audit")` · [Accessibility Audit](test-structure.md#accessibility-audit) · 606
  - `help("flow", "performance-monitoring")` · [Performance Monitoring](test-structure.md#performance-monitoring) · 609
- `help("flow", "block")` · [Block](test-structure.md#block) · 629
  - `help("flow", "platform")` · [Platform-Specific Blocks](test-structure.md#platform-specific-blocks) · 665
  - `help("flow", "branching")` · [Branching](test-structure.md#branching) · 1304
  - `help("flow", "next")` · [Block `next`](test-structure.md#block-next) · 334
- `help("flow", "step")` · [Step](test-structure.md#step) · 488
  - `help("flow", "selectors")` · [Selectors](test-structure.md#selectors) · 677
  - `help("flow", "step/options")` · [Step Options](test-structure.md#step-options) · 861
  - `help("flow", "timeout-multipliers")` · [Timeout Multipliers](test-structure.md#timeout-multipliers) · 1251
- `help("flow", "subflow")` · [Subflow (LLM text)](src/test-structure/40-subflow.llm.md) · 1083
- `help("flow", "reuse")` · [Reuse: Subflow vs Mixin vs Fixture](test-structure.md#reuse-subflow-vs-mixin-vs-fixture) · 1013
- `help("flow", "lifecycle")` · [Lifecycle: Setup & Teardown](test-structure.md#lifecycle-setup--teardown) · 1049
- `help("flow", "teardown")` · [Teardown](test-structure.md#teardown) · 643
- `help("flow", "data-driven")` · [Data-Driven Tests](test-structure.md#data-driven-tests) · 1058
- `help("flow", "variables")` · [Variables](test-structure.md#variables) · 602
  - `help("flow", "built-in")` · [Built-in variables](test-structure.md#built-in-variables) · 867
- `help("flow", "fake-data")` · [Fake Data Generators](test-structure.md#fake-data-generators) · 473
- `help("flow", "multi-app")` · [Multi-App Flows](test-structure.md#multi-app-flows) · 537
- `help("flow", "devices")` · [Devices (LLM text)](src/test-structure/120-devices/00-intro.llm.md) · 1397
  - `help("flow", "coverage")` · [Coverage Strategies](test-structure.md#coverage-strategies) · 2395
  - `help("flow", "hardware")` · [Hardware Axis (virtual / real)](test-structure.md#hardware-axis-virtual--real) · 1584
  - `help("flow", "pinning")` · [Pinning a Specific Device by Name](test-structure.md#pinning-a-specific-device-by-name) · 387
  - `help("flow", "auto-boot")` · [Auto-Boot Behaviour](test-structure.md#auto-boot-behaviour) · 404
- `help("flow", "project-config")` · [Project config (`golem.toml`)](test-structure.md#project-config-golemtoml) · 1548

### fake

- `help("fake")` · [Fake Data Generators](fake-data.md#fake-data-generators) · 1171
- `help("fake", "seeds")` · [Determinism and seeds (LLM text)](src/fake-data/10-seeds.llm.md) · 488
- `help("fake", "all-generators")` · [All generators](fake-data.md#all-generators) · 1090
- `help("fake", "simple-generators")` · [Simple generators](fake-data.md#simple-generators) · 224
  - `help("fake", "email")` · [email](fake-data.md#email) · 837
  - `help("fake", "password")` · [password](fake-data.md#password) · 217
  - `help("fake", "uuid")` · [uuid](fake-data.md#uuid) · 96
  - `help("fake", "number")` · [number](fake-data.md#number) · 165
  - `help("fake", "one_of")` · [one_of](fake-data.md#one_of) · 321
  - `help("fake", "sentence")` · [sentence](fake-data.md#sentence) · 858
  - `help("fake", "phone")` · [phone](fake-data.md#phone) · 584
- `help("fake", "structured-generators")` · [Structured generators](fake-data.md#structured-generators) · 190
  - `help("fake", "person")` · [person (LLM text)](src/fake-data/40-structured-generators/10-person.llm.md) · 2764
  - `help("fake", "address")` · [address](fake-data.md#address) · 2068
  - `help("fake", "credit_card")` · [credit_card](fake-data.md#credit_card) · 1729
  - `help("fake", "timestamp")` · [timestamp](fake-data.md#timestamp) · 1365
- `help("fake", "cross-references")` · [Cross-references](fake-data.md#cross-references) · 246

### codes

- `help("codes")` · [Error Codes (LLM text)](src/error-codes/00-intro.llm.md) · 693
- `help("codes", "registry")` · [Registry](error-codes.md#registry) · 5217
