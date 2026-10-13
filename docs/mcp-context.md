<!-- Generated from golem-cli (mcp.rs, help.rs) and docs/src. Run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-cli mcp_context`. -->
# MCP context

What an MCP client receives from `golem mcp`, on one page: the instructions, the tool list, and each answer of `help`. For golem developers; [MCP server](mcp.md) is the user guide.

| Text | Characters |
|------|-----------:|
| Instructions | 340 |
| Tool list (31 tools, as `tools/list` JSON) | 12068 |
| Both, kept in context by most clients | 12408 |

## Instructions

```text
golem (MCP server): Mobile e2e testing on iOS and Android simulators and emulators. Use when the user wants to write, edit or run an e2e test flow (.test.toml) for a mobile app, or to reproduce and debug a mobile app bug: drive the screen, read the UI tree, read crash logs. Start with session_open; help() explains steps and the flow file.
```

## Tools

### `act`

Run one step on the device: a one-line TOML inline table, e.g. { action = "tap", on_text = "Sign in" }; help("act") lists actions. Prefer on_text (text a user reads); on_accessibility_label only to test that label. A passing step goes into the draft at the cursor as written (${var} stays a reference); a failed one does not.

| Parameter | Type | Description |
|-----------|------|-------------|
| `comment` | string | Line above the step in the draft. |
| `format` | string | "toon" (default) or "json". |
| `run` | boolean | false: record without running, for a path the session does not take; it and the next step become ?. Default true. |
| `step` (required) | string |  |
| `tree` | boolean | Also return the visible tree after the step. |

### `app_logs`

App's device log (Android logcat, iOS simulator; not physical iOS): crash lines first, then newest. Answers while another operation runs.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` | string | App name or bundle id. Default: session's app. |
| `filter` | string | Only lines with this text, any case. |
| `limit` | integer | Lines besides crash lines. Default 200 newest. |
| `since` | integer | Seconds back. Default: since session opened. |

### `apps_set`

Add or replace a draft [[flow.apps]] entry. Session unchanged.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` (required) | object | { "name": "app", "bundle": "com.acme", "devices": [{ "os": "ios:latest" }] }; also permissions, install_script. Same name replaces given fields. |

### `block_begin`

Move the cursor to the end of a block; a new name adds a block at the end.

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

Set a block's next; add branches. After its steps a block takes the first branch that holds; with branches, next is ignored and no match goes to the next block in the file. Without branches: next, else the next block.

| Parameter | Type | Description |
|-----------|------|-------------|
| `block` (required) | string |  |
| `branches` | array | [{ "if_visible": "Error", "goto": "retry" }]; or if_not_visible, or if_var with equals, matches or gte. |
| `next` | string |  |

### `block_rename`

Rename a block and each next, goto and start naming it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |
| `to` (required) | string |  |

### `block_set`

Set a block's own fields: app, for_each, where (device filter), run_flow, vars, save_to, record. Its steps then become ? and ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `fields` (required) | object | { "for_each": "data", "where": { "os": "ios" } }; null removes a key. |
| `name` (required) | string |  |

### `cancel`

Stop the running operation. No teardown runs.

### `data_add`

Add a [[data]] row. A block with for_each = "data" (block_set) runs once per row; its steps read ${_each.field}.

| Parameter | Type | Description |
|-----------|------|-------------|
| `row` (required) | object | { "email": "a@b.test", "name": "Ada" } |

### `devices`

Devices in any state: platform, id, name, OS, state, live companion port.

| Parameter | Type | Description |
|-----------|------|-------------|
| `os` | string | As session_open's os. |

### `draft_run`

Run the draft on the device: no setup, no teardown, app as is. Passing steps become ✓. Stops before stop_at, at a failed step, or at the end; cursor goes there.

| Parameter | Type | Description |
|-----------|------|-------------|
| `restart` | boolean | true: from the start. Default: from the cursor. |
| `stop_at` | string | Stop before block or block:step. |

### `draft_show`

Draft as .test.toml text: the opened flow, or a new flow with block main.

### `draft_steps`

Draft steps near the cursor, or one block's: block:step (from 1), status, step, comment. Status: ✓ passed here, · not run here, ? unverified (# unverified in file), ~ stale. A change (new, edited, moved or deleted step) makes the next step ? and later steps ~. Block headers show next and branches.

| Parameter | Type | Description |
|-----------|------|-------------|
| `around` | string | "cursor" (default) or block:step. |
| `block` | string | List this block instead. |
| `context` | integer | Steps each side. Default 5. |
| `limit` | integer |  |

### `export_flow`

Check the draft as golem run does (golem.toml merged; actions, required keys, block names, branches); if it passes, write it. Returns status counts, unverified steps, run warnings.

| Parameter | Type | Description |
|-----------|------|-------------|
| `overwrite` | boolean | Needed to replace a file this session did not open or export. |
| `path` (required) | string | Relative to project, or absolute. |

### `flow_set`

Set draft [flow] fields.

| Parameter | Type | Description |
|-----------|------|-------------|
| `explicit_only` | boolean | true: golem run without a path skips it. |
| `name` | string |  |
| `start` | string | The first block. |
| `tags` | array |  |
| `vars` | object | Merged into [flow] vars; also set in session for act. |

### `help`

Docs, one piece at a time: help() lists topics, help(topic) a topic's items, help(topic, item) one item, e.g. help("act", "tap").

| Parameter | Type | Description |
|-----------|------|-------------|
| `item` | string | Action, group, section or code. |
| `topic` | string | act, selectors, flow, fake or codes. |

### `mixins_list`

Project mixins and the vars each uses. Run one with act: { action = "load_mixin", mixin = "name", vars = { … } }.

### `options_set`

Set draft [flow.options] keys, e.g. step_timeout, app_lifecycle, max_runtime, record, coverage; help("flow", "flow/options") lists all. Unknown key or wrong type refused.

| Parameter | Type | Description |
|-----------|------|-------------|
| `options` (required) | object | { "step_timeout": 8000, "app_lifecycle": "manual" }; null removes a key. |

### `probe`

What a selector matches, without acting: each visible match, which one act picks, each anchor; on a miss, which clause removed the candidates. Never fails.

| Parameter | Type | Description |
|-----------|------|-------------|
| `format` | string | "toon" (default) or "json". |
| `selector` (required) | string | { on_text = "Sign in" }; a whole step works too. |
| `timeout_ms` | integer | Poll this long while nothing matches. Default 0. |

### `screenshot`

Screen as PNG.

### `session_close`

Close session; release device.

| Parameter | Type | Description |
|-----------|------|-------------|
| `teardown` | boolean | Run flow's [[teardown]]. Default true. |

### `session_open`

Open a session on one device and app. Prefers a running device that fits; else boots one. Ends at session_close, server stop, or idle_timeout_s.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` | string | App name in golem.toml [[apps]]. |
| `boot` | boolean | Boot a device if no running one fits. Default true. |
| `break_on_failure` | boolean | With flow: open at a failed step instead of ending. |
| `bundle` | string | App bundle id. |
| `device` | string | UDID, serial, name, or part of one. |
| `flow` | string | Run this .test.toml first, as golem run does; session opens where it stops. Its [[teardown]] runs at session_close. |
| `idle_timeout_s` | integer | Idle seconds before the session ends. Default 1800. |
| `os` | string | "ios", "android", "ios:26", "ios:26+" or "ios:latest". Default: any. |
| `project` | string | Dir with golem.toml. Default: server's project. |
| `run` | boolean | With flow: false = setup only, no steps. |
| `seed` | integer | Fake data seed, as golem run --seed: replays a run's data. |
| `stop_at` | string | With flow: stop before block or block:step. |
| `teardown` | boolean | With flow: false = never run its [[teardown]]. |
| `type` | string | "phone" or "tablet". |
| `vars` | object | As golem run --var; act resolves ${name}. |

### `status`

Idle or busy, without waiting: running operation and phase, or last result. While busy only status, wait, cancel, app_logs, session_close answer.

### `step_delete`

Remove a draft step and its comment. Next step becomes ?, later ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `at` (required) | string | block:step. |

### `step_edit`

Change a draft step without running it. Only a new comment or larger timeout keeps its status; else it and the next step become ?, later ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `at` (required) | string | block:step. |
| `comment` | string | "" removes it. |
| `step` | string | Whole new step. |

### `step_move`

Move a draft step. Old place counts as a delete; at the new place it is ?.

| Parameter | Type | Description |
|-----------|------|-------------|
| `from` (required) | string | block:step. |
| `to` (required) | string | block:step after the move, counted without the moved step; one past a block's last step appends. |

### `teardown_add`

Add a step to draft [[teardown]]. Not run.

| Parameter | Type | Description |
|-----------|------|-------------|
| `comment` | string |  |
| `step` (required) | string |  |

### `teardown_delete`

Remove a [[teardown]] step. To change one: remove, then teardown_add.

| Parameter | Type | Description |
|-----------|------|-------------|
| `n` (required) | integer | Teardown step, from 1. |

### `tree`

The screen: one line per targetable element. Target by selector keys, not index.

| Parameter | Type | Description |
|-----------|------|-------------|
| `format` | string | "toon" (default) or "json". |
| `full` | boolean | Add off-screen elements: hint, never proof. |

### `wait`

Wait for the operation that answered pending; return its result (or the last result). Can answer pending again.

| Parameter | Type | Description |
|-----------|------|-------------|
| `timeout_s` | integer | Default: server's soft timeout. |

## Help

Each call that `help` answers, linked to the part in `docs/src` that it serves: the part, or the `.llm.md` file that replaces it for the LLM. An item with items of its own answers its text, then a list of them that `help` builds from their headings (+ items, counted; sub-items are the names on those items' lines). The size is the whole answer's, in characters.

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

- `help("act")` · [Actions (LLM text)](src/actions-reference/00-intro.llm.md) + 10 items, 65 sub-items · 3005
- `help("act", "interaction")` · [Interaction](src/actions-reference/10-interaction/00-intro.md) + 12 items · 449
  - `help("act", "tap")` · [`tap` — Tap an element](src/actions-reference/10-interaction/10-tap.md) · 437
  - `help("act", "double_tap")` · [`double_tap` — Double-tap an element](src/actions-reference/10-interaction/20-double_tap.md) · 175
  - `help("act", "type")` · [`type` — Type text into an element](src/actions-reference/10-interaction/30-type.md) · 489
  - `help("act", "backspace")` · [`backspace` — Delete characters](src/actions-reference/10-interaction/40-backspace.md) · 486
  - `help("act", "clear_text")` · [`clear_text` — Empty a field](src/actions-reference/10-interaction/50-clear_text.md) · 802
  - `help("act", "long_press")` · [`long_press` — Long press an element](src/actions-reference/10-interaction/60-long_press.md) · 269
  - `help("act", "swipe")` · [`swipe` — Swipe gesture](src/actions-reference/10-interaction/70-swipe.md) · 1473
  - `help("act", "scroll")` · [`scroll` — Scroll until element found](src/actions-reference/10-interaction/80-scroll.md) · 771
  - `help("act", "pinch")` · [`pinch` — Pinch zoom gesture](src/actions-reference/10-interaction/90-pinch.md) · 742
  - `help("act", "gesture")` · [`gesture` — Multi-touch gesture](src/actions-reference/10-interaction/100-gesture.md) · 806
  - `help("act", "rotate")` · [`rotate` — Rotate gesture](src/actions-reference/10-interaction/110-rotate.md) · 860
  - `help("act", "hide_keyboard")` · [`hide_keyboard` — Dismiss keyboard](src/actions-reference/10-interaction/120-hide_keyboard.md) · 463
- `help("act", "assertions")` · [Assertions](src/actions-reference/20-assertions/00-intro.md) + 3 items · 282
  - `help("act", "assert_visible")` · [`assert_visible` — Wait for / assert element exists](src/actions-reference/20-assertions/10-assert_visible.md) · 702
  - `help("act", "assert_not_visible")` · [`assert_not_visible` — Wait for / assert element absent](src/actions-reference/20-assertions/20-assert_not_visible.md) · 433
  - `help("act", "assert_alert")` · [`assert_alert` — Assert alert is displayed](src/actions-reference/20-assertions/30-assert_alert.md) · 228
- `help("act", "reading")` · [Reading](src/actions-reference/30-reading/00-intro.md) + 1 item · 106
  - `help("act", "read")` · [`read` — Read element text](src/actions-reference/30-reading/10-read.md) · 342
- `help("act", "app-lifecycle")` · [App Lifecycle](src/actions-reference/40-app-lifecycle/00-intro.md) + 4 items · 273
  - `help("act", "launch")` · [`launch` — Launch or foreground an app](src/actions-reference/40-app-lifecycle/10-launch.md) · 898
  - `help("act", "app-permissions")` · [App permissions (LLM text)](src/actions-reference/40-app-lifecycle/20-app-permissions.llm.md) · 1620
  - `help("act", "stop")` · [`stop` — Terminate an app](src/actions-reference/40-app-lifecycle/30-stop.md) · 141
  - `help("act", "clear_data")` · [`clear_data` — Clear app data](src/actions-reference/40-app-lifecycle/40-clear_data.md) · 652
- `help("act", "device-controls")` · [Device Controls](src/actions-reference/50-device-controls/00-intro.md) + 3 items · 194
  - `help("act", "set_dark_mode")` · [`set_dark_mode` — Set dark mode](src/actions-reference/50-device-controls/10-set_dark_mode.md) · 341
  - `help("act", "set_location")` · [`set_location` — Set GPS coordinates](src/actions-reference/50-device-controls/20-set_location.md) · 308
  - `help("act", "press")` · [`press` — Press hardware button](src/actions-reference/50-device-controls/30-press.md) · 477
- `help("act", "capture")` · [Capture](src/actions-reference/60-capture/00-intro.md) + 3 items · 215
  - `help("act", "screenshot")` · [`screenshot` — Take screenshot](src/actions-reference/60-capture/10-screenshot.md) · 296
  - `help("act", "recording")` · [Screen recording — per-block via `record = true`](src/actions-reference/60-capture/20-recording.md) · 545
  - `help("act", "add_media")` · [`add_media` — Push media to device](src/actions-reference/60-capture/30-add_media.md) · 535
- `help("act", "alerts")` · [Alerts](src/actions-reference/70-alerts/00-intro.md) + 2 items · 152
  - `help("act", "accept_alert")` · [`accept_alert` — Accept dialog](src/actions-reference/70-alerts/10-accept_alert.md) · 411
  - `help("act", "dismiss_alert")` · [`dismiss_alert` — Dismiss dialog](src/actions-reference/70-alerts/20-dismiss_alert.md) · 362
- `help("act", "external")` · [External](src/actions-reference/80-external/00-intro.md) + 9 items · 435
  - `help("act", "open_link")` · [`open_link` — Open URL or deep link](src/actions-reference/80-external/10-open_link.md) · 201
  - `help("act", "push_notification")` · [`push_notification` — Deliver a push to the app under test (LLM text)](src/actions-reference/80-external/20-push_notification.llm.md) · 948
  - `help("act", "bash")` · [`bash` — Run shell command](src/actions-reference/80-external/30-bash.md) · 310
  - `help("act", "run")` · [`run` — Run project script](src/actions-reference/80-external/40-run.md) · 635
  - `help("act", "create_inbox")` · [`create_inbox` — Provision a disposable email inbox](src/actions-reference/80-external/50-create_inbox.md) · 1329
  - `help("act", "await_email")` · [`await_email` — Poll IMAP inbox](src/actions-reference/80-external/60-await_email.md) · 1314
  - `help("act", "load_fixture")` · [`load_fixture` — Load fixture data](src/actions-reference/80-external/70-load_fixture.md) · 543
  - `help("act", "load_mixin")` · [`load_mixin` — Inline a reusable step sequence](src/actions-reference/80-external/80-load_mixin.md) · 707
  - `help("act", "http")` · [`get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests](src/actions-reference/80-external/90-http.md) · 668
- `help("act", "browser")` · [Browser (LLM text)](src/actions-reference/90-browser/00-intro.llm.md) + 20 items · 2227
  - `help("act", "browse_navigate")` · [`browse_navigate` — Load a URL (LLM text)](src/actions-reference/90-browser/10-browse_navigate.llm.md) · 805
  - `help("act", "browse_tap")` · [`browse_tap` — Click an element](src/actions-reference/90-browser/20-browse_tap.md) · 209
  - `help("act", "browse_type")` · [`browse_type` — Type into a field](src/actions-reference/90-browser/30-browse_type.md) · 400
  - `help("act", "browse_read")` · [`browse_read` — Read text or an attribute into a variable](src/actions-reference/90-browser/40-browse_read.md) · 590
  - `help("act", "browse_screenshot")` · [`browse_screenshot` — Capture the tab](src/actions-reference/90-browser/50-browse_screenshot.md) · 310
  - `help("act", "browse_assert_exists")` · [`browse_assert_exists` — The element is in the DOM](src/actions-reference/90-browser/60-browse_assert_exists.md) · 239
  - `help("act", "browse_assert_not_exists")` · [`browse_assert_not_exists` — The element is not in the DOM](src/actions-reference/90-browser/70-browse_assert_not_exists.md) · 287
  - `help("act", "browse_assert_text")` · [`browse_assert_text` — The element says what you expect](src/actions-reference/90-browser/80-browse_assert_text.md) · 795
  - `help("act", "browse_wait_exists")` · [`browse_wait_exists` — Wait for an element to appear](src/actions-reference/90-browser/90-browse_wait_exists.md) · 331
  - `help("act", "browse_wait_not_exists")` · [`browse_wait_not_exists` — Wait for an element to disappear](src/actions-reference/90-browser/100-browse_wait_not_exists.md) · 317
  - `help("act", "browse_scroll_by")` · [`browse_scroll_by` — Scroll by a distance](src/actions-reference/90-browser/110-browse_scroll_by.md) · 586
  - `help("act", "browse_scroll_to")` · [`browse_scroll_to` — Bring an element into view](src/actions-reference/90-browser/120-browse_scroll_to.md) · 206
  - `help("act", "browse_select")` · [`browse_select` — Choose an option in a `<select>`](src/actions-reference/90-browser/130-browse_select.md) · 536
  - `help("act", "browse_execute_js")` · [`browse_execute_js` — Run JavaScript in the page (LLM text)](src/actions-reference/90-browser/140-browse_execute_js.llm.md) · 831
  - `help("act", "browse_mcp_list_tools")` · [`browse_mcp_list_tools` — List the page's WebMCP tools](src/actions-reference/90-browser/150-browse_mcp_list_tools.md) · 417
  - `help("act", "browse_mcp_call")` · [`browse_mcp_call` — Call a WebMCP tool](src/actions-reference/90-browser/160-browse_mcp_call.md) · 858
  - `help("act", "browse_cookies")` · [`browse_set_cookie` / `browse_get_cookie` — Cookies](src/actions-reference/90-browser/170-browse_cookies.md) · 732
  - `help("act", "browse_local_storage")` · [`browse_set_local_storage` / `browse_get_local_storage` — Local storage](src/actions-reference/90-browser/180-browse_local_storage.md) · 647
  - `help("act", "browse_session_storage")` · [`browse_set_session_storage` / `browse_get_session_storage` — Session storage](src/actions-reference/90-browser/190-browse_session_storage.md) · 302
  - `help("act", "browse_close")` · [`browse_close` — Close a tab early](src/actions-reference/90-browser/200-browse_close.md) · 206
- `help("act", "flow-control")` · [Flow Control](src/actions-reference/100-flow-control/00-intro.md) + 1 item · 104
  - `help("act", "fail")` · [`fail` — Fail the flow immediately](src/actions-reference/100-flow-control/10-fail.md) · 455

### selectors

- `help("selectors")` · [Selectors](src/selectors/00-intro.md) + 10 items, 8 sub-items · 1200
- `help("selectors", "two-syntaxes")` · [Two syntaxes](src/selectors/10-two-syntaxes.md) · 359
- `help("selectors", "core")` · [Core selectors](src/selectors/20-core/00-intro.md) + 1 item · 1116
  - `help("selectors", "prefer-text")` · [Prefer visible `text`; use `accessibility_label` sparingly](src/selectors/20-core/10-prefer-text.md) · 664
- `help("selectors", "state-filters")` · [State filters](src/selectors/30-state-filters.md) · 296
- `help("selectors", "traits")` · [Traits](src/selectors/40-traits.md) · 579
- `help("selectors", "relational")` · [Relational (positional) selectors](src/selectors/50-relational.md) · 1163
- `help("selectors", "containment")` · [Geometric containment: `contains` / `inside`](src/selectors/60-containment/00-intro.md) + 1 item · 894
  - `help("selectors", "min_matches")` · [`min_matches` — the container of *repeated* items](src/selectors/60-containment/10-min_matches.md) · 1142
- `help("selectors", "chaining")` · [Nesting and chaining (LLM text)](src/selectors/70-chaining.llm.md) · 648
- `help("selectors", "occlusion")` · [Occlusion-aware tapping](src/selectors/80-occlusion.md) · 1045
- `help("selectors", "within")` · [`within` (scoping a scroll)](src/selectors/90-within.md) · 700
- `help("selectors", "canvas")` · [Canvas-rendered UI (Compose, Compose Multiplatform, Flutter) (LLM text)](src/selectors/100-canvas/00-intro.llm.md) + 6 items · 2407
  - `help("selectors", "version-floors")` · [Version floors](src/selectors/100-canvas/10-version-floors.md) · 201
  - `help("selectors", "annotations")` · [Which annotation reaches golem](src/selectors/100-canvas/20-annotations.md) · 1264
  - `help("selectors", "element-types")` · [Element types](src/selectors/100-canvas/30-element-types.md) · 1086
  - `help("selectors", "bounds")` · [Bounds and visibility](src/selectors/100-canvas/40-bounds.md) · 1010
  - `help("selectors", "merged-semantics")` · [Merged semantics](src/selectors/100-canvas/50-merged-semantics.md) · 1250
  - `help("selectors", "flutter")` · [Flutter](src/selectors/100-canvas/60-flutter.md) · 164

### flow

- `help("flow")` · [Test Structure](src/test-structure/00-intro.md) + 13 items, 15 sub-items · 1207
- `help("flow", "flow")` · [Flow](src/test-structure/10-flow/00-intro.md) + 4 items · 1415
  - `help("flow", "launch-permissions")` · [Launch-time permissions](src/test-structure/10-flow/05-launch-permissions.md) · 827
  - `help("flow", "flow/options")` · [Flow Options](src/test-structure/10-flow/10-options.md) · 1984
  - `help("flow", "accessibility-audit")` · [Accessibility Audit](src/test-structure/10-flow/20-accessibility-audit.md) · 602
  - `help("flow", "performance-monitoring")` · [Performance Monitoring](src/test-structure/10-flow/30-performance-monitoring.md) · 609
- `help("flow", "block")` · [Block](src/test-structure/20-block/00-intro.md) + 3 items · 743
  - `help("flow", "platform")` · [Platform-Specific Blocks](src/test-structure/20-block/10-platform.md) · 663
  - `help("flow", "branching")` · [Branching](src/test-structure/20-block/20-branching.md) · 1292
  - `help("flow", "next")` · [Block `next`](src/test-structure/20-block/30-next.md) · 334
- `help("flow", "step")` · [Step](src/test-structure/30-step/00-intro.md) + 3 items · 614
  - `help("flow", "selectors")` · [Selectors](src/test-structure/30-step/10-selectors.md) · 675
  - `help("flow", "step/options")` · [Step Options](src/test-structure/30-step/20-options.md) · 851
  - `help("flow", "timeout-multipliers")` · [Timeout Multipliers](src/test-structure/30-step/30-timeout-multipliers.md) · 1251
- `help("flow", "subflow")` · [Subflow (LLM text)](src/test-structure/40-subflow.llm.md) · 1083
- `help("flow", "reuse")` · [Reuse: Subflow vs Mixin vs Fixture](src/test-structure/50-reuse.md) · 1011
- `help("flow", "lifecycle")` · [Lifecycle: Setup & Teardown](src/test-structure/60-lifecycle.md) · 1037
- `help("flow", "teardown")` · [Teardown](src/test-structure/70-teardown.md) · 641
- `help("flow", "data-driven")` · [Data-Driven Tests](src/test-structure/80-data-driven.md) · 1050
- `help("flow", "variables")` · [Variables](src/test-structure/90-variables/00-intro.md) + 1 item · 644
  - `help("flow", "built-in")` · [Built-in variables](src/test-structure/90-variables/10-built-in.md) · 861
- `help("flow", "fake-data")` · [Fake Data Generators](src/test-structure/100-fake-data.md) · 471
- `help("flow", "multi-app")` · [Multi-App Flows](src/test-structure/110-multi-app.md) · 537
- `help("flow", "devices")` · [Devices (LLM text)](src/test-structure/120-devices/00-intro.llm.md) + 4 items · 1509
  - `help("flow", "coverage")` · [Coverage Strategies](src/test-structure/120-devices/10-coverage.md) · 2379
  - `help("flow", "hardware")` · [Hardware Axis (virtual / real)](src/test-structure/120-devices/20-hardware.md) · 1578
  - `help("flow", "pinning")` · [Pinning a Specific Device by Name](src/test-structure/120-devices/30-pinning.md) · 387
  - `help("flow", "auto-boot")` · [Auto-Boot Behaviour](src/test-structure/120-devices/40-auto-boot.md) · 402
- `help("flow", "project-config")` · [Project config (`golem.toml`)](src/test-structure/130-project-config.md) · 1544

### fake

- `help("fake")` · [Fake Data Generators](src/fake-data/00-intro.md) + 5 items, 11 sub-items · 1257
- `help("fake", "seeds")` · [Determinism and seeds (LLM text)](src/fake-data/10-seeds.llm.md) · 576
- `help("fake", "all-generators")` · [All generators](src/fake-data/20-all-generators.md) · 1088
- `help("fake", "simple-generators")` · [Simple generators](src/fake-data/30-simple-generators/00-intro.md) + 7 items · 365
  - `help("fake", "email")` · [email](src/fake-data/30-simple-generators/10-email.md) · 825
  - `help("fake", "password")` · [password](src/fake-data/30-simple-generators/20-password.md) · 213
  - `help("fake", "uuid")` · [uuid](src/fake-data/30-simple-generators/30-uuid.md) · 94
  - `help("fake", "number")` · [number](src/fake-data/30-simple-generators/40-number.md) · 163
  - `help("fake", "one_of")` · [one_of](src/fake-data/30-simple-generators/50-one_of.md) · 319
  - `help("fake", "sentence")` · [sentence](src/fake-data/30-simple-generators/60-sentence.md) · 820
  - `help("fake", "phone")` · [phone](src/fake-data/30-simple-generators/70-phone.md) · 572
- `help("fake", "structured-generators")` · [Structured generators](src/fake-data/40-structured-generators/00-intro.md) + 4 items · 355
  - `help("fake", "person")` · [person (LLM text)](src/fake-data/40-structured-generators/10-person.llm.md) · 2749
  - `help("fake", "address")` · [address](src/fake-data/40-structured-generators/20-address.md) · 2028
  - `help("fake", "credit_card")` · [credit_card](src/fake-data/40-structured-generators/30-credit_card.md) · 1719
  - `help("fake", "timestamp")` · [timestamp](src/fake-data/40-structured-generators/40-timestamp.md) · 1351
- `help("fake", "cross-references")` · [Cross-references](src/fake-data/50-cross-references.md) · 246

### codes

- `help("codes")` · [Error Codes (LLM text)](src/error-codes/00-intro.llm.md) + 1 item · 789
- `help("codes", "registry")` · [Registry](src/error-codes/10-registry.md) · 5215
