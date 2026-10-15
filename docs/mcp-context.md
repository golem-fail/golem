<!-- Generated from golem-cli (mcp.rs, help.rs) and docs/src. Run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-cli mcp_context`. -->
# MCP context

What an MCP client receives from `golem mcp`, on one page: the instructions, the tool list, and each answer of `help`. For golem developers; [MCP server](mcp.md) is the user guide.

| Text | Characters |
|------|-----------:|
| Instructions | 477 |
| Tool list (31 tools, as `tools/list` JSON) | 12993 |
| Both, kept in context by most clients | 13470 |

## Instructions

```text
golem (MCP server): Mobile e2e testing on iOS and Android simulators and emulators. Use when the user wants to write, edit or run an e2e test flow (.test.toml) for a mobile app, or to reproduce and debug a mobile app bug: drive the screen, read the UI tree, read crash logs. Start with session_open; help() explains steps and the flow file. To write a flow, record steps with act, arrange them with block_* and step_*, and write the file only with export_flow, which checks it.
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

<details><summary>As <code>tools/list</code> sends it · 849 chars</summary>

```json
{"name":"act","description":"Run one step on the device: a one-line TOML inline table, e.g. { action = \"tap\", on_text = \"Sign in\" }; help(\"act\") lists actions. Prefer on_text (text a user reads); on_accessibility_label only to test that label. A passing step goes into the draft at the cursor as written (${var} stays a reference); a failed one does not.","inputSchema":{"properties":{"comment":{"description":"Line above the step in the draft.","type":"string"},"format":{"description":"\"toon\" (default) or \"json\".","type":"string"},"run":{"description":"false: record without running, for a path the session does not take; it and the next step become ?. Default true.","type":"boolean"},"step":{"type":"string"},"tree":{"description":"Also return the visible tree after the step.","type":"boolean"}},"required":["step"],"type":"object"}}
```

</details>

### `app_logs`

App's device log (Android logcat, iOS simulator; not physical iOS): crash lines first, then newest. Answers while another operation runs.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` | string | App name or bundle id. Default: session's app. |
| `filter` | string | Only lines with this text, any case. |
| `limit` | integer | Lines besides crash lines. Default 200 newest. |
| `since` | integer | Seconds back. Default: since session opened. |

<details><summary>As <code>tools/list</code> sends it · 589 chars</summary>

```json
{"name":"app_logs","description":"App's device log (Android logcat, iOS simulator; not physical iOS): crash lines first, then newest. Answers while another operation runs.","inputSchema":{"properties":{"app":{"description":"App name or bundle id. Default: session's app.","type":"string"},"filter":{"description":"Only lines with this text, any case.","type":"string"},"limit":{"description":"Lines besides crash lines. Default 200 newest.","minimum":0,"type":"integer"},"since":{"description":"Seconds back. Default: since session opened.","minimum":0,"type":"integer"}},"type":"object"}}
```

</details>

### `apps_set`

Add or replace a draft [[flow.apps]] entry. Session unchanged.

| Parameter | Type | Description |
|-----------|------|-------------|
| `app` (required) | object | { "name": "app", "bundle": "com.acme", "devices": [{ "os": "ios:latest" }] }; also permissions, install_script. Same name replaces given fields. |

<details><summary>As <code>tools/list</code> sends it · 391 chars</summary>

```json
{"name":"apps_set","description":"Add or replace a draft [[flow.apps]] entry. Session unchanged.","inputSchema":{"properties":{"app":{"additionalProperties":true,"description":"{ \"name\": \"app\", \"bundle\": \"com.acme\", \"devices\": [{ \"os\": \"ios:latest\" }] }; also permissions, install_script. Same name replaces given fields.","type":"object"}},"required":["app"],"type":"object"}}
```

</details>

### `block_begin`

Move the cursor to the end of a block; a new name adds a block at the end.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |
| `next` | string | Its next block. |

<details><summary>As <code>tools/list</code> sends it · 262 chars</summary>

```json
{"name":"block_begin","description":"Move the cursor to the end of a block; a new name adds a block at the end.","inputSchema":{"properties":{"name":{"type":"string"},"next":{"description":"Its next block.","type":"string"}},"required":["name"],"type":"object"}}
```

</details>

### `block_delete`

Remove a block and its steps. Refused while a next, goto or start names it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |

<details><summary>As <code>tools/list</code> sends it · 207 chars</summary>

```json
{"name":"block_delete","description":"Remove a block and its steps. Refused while a next, goto or start names it.","inputSchema":{"properties":{"name":{"type":"string"}},"required":["name"],"type":"object"}}
```

</details>

### `block_link`

Set a block's next; add branches. After its steps a block takes the first branch that holds; with branches, next is ignored and no match goes to the next block in the file. Without branches: next, else the next block.

| Parameter | Type | Description |
|-----------|------|-------------|
| `block` (required) | string |  |
| `branches` | array | [{ "if_visible": "Error", "goto": "retry" }]; or if_not_visible, or if_var with equals, matches or gte. |
| `next` | string |  |

<details><summary>As <code>tools/list</code> sends it · 530 chars</summary>

```json
{"name":"block_link","description":"Set a block's next; add branches. After its steps a block takes the first branch that holds; with branches, next is ignored and no match goes to the next block in the file. Without branches: next, else the next block.","inputSchema":{"properties":{"block":{"type":"string"},"branches":{"description":"[{ \"if_visible\": \"Error\", \"goto\": \"retry\" }]; or if_not_visible, or if_var with equals, matches or gte.","type":"array"},"next":{"type":"string"}},"required":["block"],"type":"object"}}
```

</details>

### `block_rename`

Rename a block and each next, goto and start naming it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` (required) | string |  |
| `to` (required) | string |  |

<details><summary>As <code>tools/list</code> sends it · 215 chars</summary>

```json
{"name":"block_rename","description":"Rename a block and each next, goto and start naming it.","inputSchema":{"properties":{"name":{"type":"string"},"to":{"type":"string"}},"required":["name","to"],"type":"object"}}
```

</details>

### `block_set`

Set a block's own fields: for_each, where (device filter), record, max_iterations, run_flow and its vars and save_to. Its steps then become ? and ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `fields` (required) | object | { "for_each": "data", "where": { "os": "ios" } }; null removes a key. |
| `name` (required) | string |  |

<details><summary>As <code>tools/list</code> sends it · 437 chars</summary>

```json
{"name":"block_set","description":"Set a block's own fields: for_each, where (device filter), record, max_iterations, run_flow and its vars and save_to. Its steps then become ? and ~.","inputSchema":{"properties":{"fields":{"additionalProperties":true,"description":"{ \"for_each\": \"data\", \"where\": { \"os\": \"ios\" } }; null removes a key.","type":"object"},"name":{"type":"string"}},"required":["name","fields"],"type":"object"}}
```

</details>

### `cancel`

Stop the running operation. No teardown runs.

<details><summary>As <code>tools/list</code> sends it · 111 chars</summary>

```json
{"name":"cancel","description":"Stop the running operation. No teardown runs.","inputSchema":{"type":"object"}}
```

</details>

### `data_add`

Add a [[data]] row. A block with for_each = "data" (block_set) runs once per row; its steps read ${_each.field}.

| Parameter | Type | Description |
|-----------|------|-------------|
| `row` (required) | object | { "email": "a@b.test", "name": "Ada" } |

<details><summary>As <code>tools/list</code> sends it · 331 chars</summary>

```json
{"name":"data_add","description":"Add a [[data]] row. A block with for_each = \"data\" (block_set) runs once per row; its steps read ${_each.field}.","inputSchema":{"properties":{"row":{"additionalProperties":true,"description":"{ \"email\": \"a@b.test\", \"name\": \"Ada\" }","type":"object"}},"required":["row"],"type":"object"}}
```

</details>

### `devices`

Devices in any state: platform, id, name, OS, state, live companion port.

| Parameter | Type | Description |
|-----------|------|-------------|
| `os` | string | As session_open's os. |

<details><summary>As <code>tools/list</code> sends it · 216 chars</summary>

```json
{"name":"devices","description":"Devices in any state: platform, id, name, OS, state, live companion port.","inputSchema":{"properties":{"os":{"description":"As session_open's os.","type":"string"}},"type":"object"}}
```

</details>

### `draft_run`

Run the draft on the device: no setup, no teardown, app as is. Passing steps become ✓. Stops before stop_at, at a failed step, or at the end; cursor goes there.

| Parameter | Type | Description |
|-----------|------|-------------|
| `restart` | boolean | true: from the start. Default: from the cursor. |
| `stop_at` | string | Stop before block or block:step. |

<details><summary>As <code>tools/list</code> sends it · 414 chars</summary>

```json
{"name":"draft_run","description":"Run the draft on the device: no setup, no teardown, app as is. Passing steps become ✓. Stops before stop_at, at a failed step, or at the end; cursor goes there.","inputSchema":{"properties":{"restart":{"description":"true: from the start. Default: from the cursor.","type":"boolean"},"stop_at":{"description":"Stop before block or block:step.","type":"string"}},"type":"object"}}
```

</details>

### `draft_show`

Draft as .test.toml text: the opened flow, or a new flow with block main.

<details><summary>As <code>tools/list</code> sends it · 143 chars</summary>

```json
{"name":"draft_show","description":"Draft as .test.toml text: the opened flow, or a new flow with block main.","inputSchema":{"type":"object"}}
```

</details>

### `draft_steps`

Draft steps near the cursor, or one block's: block:step (from 1), status, step, comment. Status: ✓ passed here, · not run here, ? unverified (# unverified in file), ~ stale. A change (new, edited, moved or deleted step) makes the next step ? and later steps ~. Block headers show next and branches.

| Parameter | Type | Description |
|-----------|------|-------------|
| `around` | string | "cursor" (default) or block:step. |
| `block` | string | List this block instead. |
| `context` | integer | Steps each side. Default 5. |
| `limit` | integer |  |

<details><summary>As <code>tools/list</code> sends it · 654 chars</summary>

```json
{"name":"draft_steps","description":"Draft steps near the cursor, or one block's: block:step (from 1), status, step, comment. Status: ✓ passed here, · not run here, ? unverified (# unverified in file), ~ stale. A change (new, edited, moved or deleted step) makes the next step ? and later steps ~. Block headers show next and branches.","inputSchema":{"properties":{"around":{"description":"\"cursor\" (default) or block:step.","type":"string"},"block":{"description":"List this block instead.","type":"string"},"context":{"description":"Steps each side. Default 5.","minimum":0,"type":"integer"},"limit":{"minimum":0,"type":"integer"}},"type":"object"}}
```

</details>

### `export_flow`

Check the draft as golem run does (golem.toml merged; actions, required keys, block names, branches); if it passes, write it. Returns status counts, unverified steps, run warnings.

| Parameter | Type | Description |
|-----------|------|-------------|
| `fixtures` | array | From a fixture hint: put ${x.key} in place of the value. |
| `mixins` | array | From a mixin hint: put a load_mixin step in place of the matching steps. |
| `overwrite` | boolean | Needed to replace a file this session did not open or export. |
| `path` (required) | string | Relative to project, or absolute. |

<details><summary>As <code>tools/list</code> sends it · 1098 chars</summary>

```json
{"name":"export_flow","description":"Check the draft as golem run does (golem.toml merged; actions, required keys, block names, branches); if it passes, write it. Returns status counts, unverified steps, run warnings.","inputSchema":{"$defs":{"FixtureAt":{"properties":{"at":{"description":"block:step.","type":"string"},"key":{"description":"fixture.key","type":"string"}},"required":["at","key"],"type":"object"},"MixinAt":{"properties":{"at":{"description":"First step, block:step.","type":"string"},"mixin":{"type":"string"}},"required":["at","mixin"],"type":"object"}},"properties":{"fixtures":{"description":"From a fixture hint: put ${x.key} in place of the value.","items":{"$ref":"#/$defs/FixtureAt"},"type":"array"},"mixins":{"description":"From a mixin hint: put a load_mixin step in place of the matching steps.","items":{"$ref":"#/$defs/MixinAt"},"type":"array"},"overwrite":{"description":"Needed to replace a file this session did not open or export.","type":"boolean"},"path":{"description":"Relative to project, or absolute.","type":"string"}},"required":["path"],"type":"object"}}
```

</details>

### `flow_set`

Set draft [flow] fields.

| Parameter | Type | Description |
|-----------|------|-------------|
| `explicit_only` | boolean | true: golem run without a path skips it. |
| `name` | string |  |
| `start` | string | The first block. |
| `tags` | array |  |
| `vars` | object | Merged into [flow] vars; also set in session for act. |

<details><summary>As <code>tools/list</code> sends it · 469 chars</summary>

```json
{"name":"flow_set","description":"Set draft [flow] fields.","inputSchema":{"properties":{"explicit_only":{"description":"true: golem run without a path skips it.","type":"boolean"},"name":{"type":"string"},"start":{"description":"The first block.","type":"string"},"tags":{"items":{"type":"string"},"type":"array"},"vars":{"additionalProperties":{"type":"string"},"description":"Merged into [flow] vars; also set in session for act.","type":"object"}},"type":"object"}}
```

</details>

### `help`

Docs, one piece at a time: help() lists topics, help(topic) a topic's items, help(topic, item) one item, e.g. help("act", "tap").

| Parameter | Type | Description |
|-----------|------|-------------|
| `item` | string | Action, group, section or code. |
| `topic` | string | act, selectors, flow, fake or codes. |

<details><summary>As <code>tools/list</code> sends it · 364 chars</summary>

```json
{"name":"help","description":"Docs, one piece at a time: help() lists topics, help(topic) a topic's items, help(topic, item) one item, e.g. help(\"act\", \"tap\").","inputSchema":{"properties":{"item":{"description":"Action, group, section or code.","type":"string"},"topic":{"description":"act, selectors, flow, fake or codes.","type":"string"}},"type":"object"}}
```

</details>

### `options_set`

Set draft [flow.options] keys, e.g. step_timeout, app_lifecycle, max_runtime, record, coverage; help("flow", "flow/options") lists all. Unknown key or wrong type refused.

| Parameter | Type | Description |
|-----------|------|-------------|
| `options` (required) | object | { "step_timeout": 8000, "app_lifecycle": "manual" }; null removes a key. |

<details><summary>As <code>tools/list</code> sends it · 434 chars</summary>

```json
{"name":"options_set","description":"Set draft [flow.options] keys, e.g. step_timeout, app_lifecycle, max_runtime, record, coverage; help(\"flow\", \"flow/options\") lists all. Unknown key or wrong type refused.","inputSchema":{"properties":{"options":{"additionalProperties":true,"description":"{ \"step_timeout\": 8000, \"app_lifecycle\": \"manual\" }; null removes a key.","type":"object"}},"required":["options"],"type":"object"}}
```

</details>

### `probe`

What a selector matches, without acting: each visible match, which one act picks, each anchor; on a miss, which clause removed the candidates. Never fails.

| Parameter | Type | Description |
|-----------|------|-------------|
| `format` | string | "toon" (default) or "json". |
| `selector` (required) | string | { on_text = "Sign in" }; a whole step works too. |
| `timeout_ms` | integer | Poll this long while nothing matches. Default 0. |

<details><summary>As <code>tools/list</code> sends it · 539 chars</summary>

```json
{"name":"probe","description":"What a selector matches, without acting: each visible match, which one act picks, each anchor; on a miss, which clause removed the candidates. Never fails.","inputSchema":{"properties":{"format":{"description":"\"toon\" (default) or \"json\".","type":"string"},"selector":{"description":"{ on_text = \"Sign in\" }; a whole step works too.","type":"string"},"timeout_ms":{"description":"Poll this long while nothing matches. Default 0.","minimum":0,"type":"integer"}},"required":["selector"],"type":"object"}}
```

</details>

### `reuse_list`

Project files a flow can reuse, each with how to use it: mixins and the vars they use, fixtures and their keys, subflows and the vars they read. name: one mixin's steps.

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` | string | A mixin's name: list its steps. |

<details><summary>As <code>tools/list</code> sends it · 327 chars</summary>

```json
{"name":"reuse_list","description":"Project files a flow can reuse, each with how to use it: mixins and the vars they use, fixtures and their keys, subflows and the vars they read. name: one mixin's steps.","inputSchema":{"properties":{"name":{"description":"A mixin's name: list its steps.","type":"string"}},"type":"object"}}
```

</details>

### `screenshot`

Screen as PNG.

<details><summary>As <code>tools/list</code> sends it · 84 chars</summary>

```json
{"name":"screenshot","description":"Screen as PNG.","inputSchema":{"type":"object"}}
```

</details>

### `session_close`

Close session; release device.

| Parameter | Type | Description |
|-----------|------|-------------|
| `teardown` | boolean | Run flow's [[teardown]]. Default true. |

<details><summary>As <code>tools/list</code> sends it · 203 chars</summary>

```json
{"name":"session_close","description":"Close session; release device.","inputSchema":{"properties":{"teardown":{"description":"Run flow's [[teardown]]. Default true.","type":"boolean"}},"type":"object"}}
```

</details>

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

<details><summary>As <code>tools/list</code> sends it · 1690 chars</summary>

```json
{"name":"session_open","description":"Open a session on one device and app. Prefers a running device that fits; else boots one. Ends at session_close, server stop, or idle_timeout_s.","inputSchema":{"properties":{"app":{"description":"App name in golem.toml [[apps]].","type":"string"},"boot":{"description":"Boot a device if no running one fits. Default true.","type":"boolean"},"break_on_failure":{"description":"With flow: open at a failed step instead of ending.","type":"boolean"},"bundle":{"description":"App bundle id.","type":"string"},"device":{"description":"UDID, serial, name, or part of one.","type":"string"},"flow":{"description":"Run this .test.toml first, as golem run does; session opens where it stops. Its [[teardown]] runs at session_close.","type":"string"},"idle_timeout_s":{"description":"Idle seconds before the session ends. Default 1800.","minimum":0,"type":"integer"},"os":{"description":"\"ios\", \"android\", \"ios:26\", \"ios:26+\" or \"ios:latest\". Default: any.","type":"string"},"project":{"description":"Dir with golem.toml. Default: server's project.","type":"string"},"run":{"description":"With flow: false = setup only, no steps.","type":"boolean"},"seed":{"description":"Fake data seed, as golem run --seed: replays a run's data.","minimum":0,"type":"integer"},"stop_at":{"description":"With flow: stop before block or block:step.","type":"string"},"teardown":{"description":"With flow: false = never run its [[teardown]].","type":"boolean"},"type":{"description":"\"phone\" or \"tablet\".","type":"string"},"vars":{"additionalProperties":{"type":"string"},"description":"As golem run --var; act resolves ${name}.","type":"object"}},"type":"object"}}
```

</details>

### `status`

Idle or busy, without waiting: running operation and phase, or last result. While busy only status, wait, cancel, app_logs, session_close answer.

<details><summary>As <code>tools/list</code> sends it · 211 chars</summary>

```json
{"name":"status","description":"Idle or busy, without waiting: running operation and phase, or last result. While busy only status, wait, cancel, app_logs, session_close answer.","inputSchema":{"type":"object"}}
```

</details>

### `step_delete`

Remove a draft step and its comment. Next step becomes ?, later ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `at` (required) | string | block:step. |

<details><summary>As <code>tools/list</code> sends it · 221 chars</summary>

```json
{"name":"step_delete","description":"Remove a draft step and its comment. Next step becomes ?, later ~.","inputSchema":{"properties":{"at":{"description":"block:step.","type":"string"}},"required":["at"],"type":"object"}}
```

</details>

### `step_edit`

Change a draft step without running it. Only a new comment or larger timeout keeps its status; else it and the next step become ?, later ~.

| Parameter | Type | Description |
|-----------|------|-------------|
| `at` (required) | string | block:step. |
| `comment` | string | "" removes it. |
| `inline` | boolean | On a load_mixin step: copy the mixin's steps into the draft in its place, to edit them here. |
| `step` | string | Whole new step. |

<details><summary>As <code>tools/list</code> sends it · 547 chars</summary>

```json
{"name":"step_edit","description":"Change a draft step without running it. Only a new comment or larger timeout keeps its status; else it and the next step become ?, later ~.","inputSchema":{"properties":{"at":{"description":"block:step.","type":"string"},"comment":{"description":"\"\" removes it.","type":"string"},"inline":{"description":"On a load_mixin step: copy the mixin's steps into the draft in its place, to edit them here.","type":"boolean"},"step":{"description":"Whole new step.","type":"string"}},"required":["at"],"type":"object"}}
```

</details>

### `step_move`

Move a draft step. Old place counts as a delete; at the new place it is ?.

| Parameter | Type | Description |
|-----------|------|-------------|
| `from` (required) | string | block:step. |
| `to` (required) | string | block:step after the move, counted without the moved step; one past a block's last step appends. |

<details><summary>As <code>tools/list</code> sends it · 372 chars</summary>

```json
{"name":"step_move","description":"Move a draft step. Old place counts as a delete; at the new place it is ?.","inputSchema":{"properties":{"from":{"description":"block:step.","type":"string"},"to":{"description":"block:step after the move, counted without the moved step; one past a block's last step appends.","type":"string"}},"required":["from","to"],"type":"object"}}
```

</details>

### `teardown_add`

Add a step to draft [[teardown]]. Not run.

| Parameter | Type | Description |
|-----------|------|-------------|
| `comment` | string |  |
| `step` (required) | string |  |

<details><summary>As <code>tools/list</code> sends it · 202 chars</summary>

```json
{"name":"teardown_add","description":"Add a step to draft [[teardown]]. Not run.","inputSchema":{"properties":{"comment":{"type":"string"},"step":{"type":"string"}},"required":["step"],"type":"object"}}
```

</details>

### `teardown_delete`

Remove a [[teardown]] step. To change one: remove, then teardown_add.

| Parameter | Type | Description |
|-----------|------|-------------|
| `n` (required) | integer | Teardown step, from 1. |

<details><summary>As <code>tools/list</code> sends it · 250 chars</summary>

```json
{"name":"teardown_delete","description":"Remove a [[teardown]] step. To change one: remove, then teardown_add.","inputSchema":{"properties":{"n":{"description":"Teardown step, from 1.","minimum":0,"type":"integer"}},"required":["n"],"type":"object"}}
```

</details>

### `tree`

The screen: one line per targetable element. Target by selector keys, not index.

| Parameter | Type | Description |
|-----------|------|-------------|
| `format` | string | "toon" (default) or "json". |
| `full` | boolean | Add off-screen elements: hint, never proof. |

<details><summary>As <code>tools/list</code> sends it · 320 chars</summary>

```json
{"name":"tree","description":"The screen: one line per targetable element. Target by selector keys, not index.","inputSchema":{"properties":{"format":{"description":"\"toon\" (default) or \"json\".","type":"string"},"full":{"description":"Add off-screen elements: hint, never proof.","type":"boolean"}},"type":"object"}}
```

</details>

### `wait`

Wait for the operation that answered pending; return its result (or the last result). Can answer pending again.

| Parameter | Type | Description |
|-----------|------|-------------|
| `timeout_s` | integer | Default: server's soft timeout. |

<details><summary>As <code>tools/list</code> sends it · 281 chars</summary>

```json
{"name":"wait","description":"Wait for the operation that answered pending; return its result (or the last result). Can answer pending again.","inputSchema":{"properties":{"timeout_s":{"description":"Default: server's soft timeout.","minimum":0,"type":"integer"}},"type":"object"}}
```

</details>

## Help

Each call that `help` answers, folded, with the answer as the LLM gets it and the part in `docs/src` that it serves: the part, or the `.llm.md` file that replaces it for the LLM. An item with items of its own answers its text, then a list of them that `help` builds from their headings (+ items, counted; sub-items are the names on those items' lines). Sizes are in characters.

`help()`:

```text
help(topic) lists a topic's items; help(topic, item) gives one item.

Topics:
- act: the step notation and every action, in groups
- selectors: how a step finds its element: text, label, anchors, traits
- flow: the .test.toml file: blocks, branches, steps, vars, data, teardown, devices
- install: the app install script: its arguments, install_script forms, templates (golem install-script --print)
- fake: fake data such as ${fake:email}, person, address, credit_card
- codes: failure codes such as EF404: what each means and its fix
```

### act

<details><summary><code>help("act")</code> · Actions · 2605 chars · + 10 items, 42 sub-items</summary>

Source: [src/actions-reference/00-intro.llm.md](src/actions-reference/00-intro.llm.md) (LLM text)

````text
# Actions

A step is one TOML inline table on one line. It is the same text as a step in a flow file:

```toml
{ action = "tap", on_text = "Sign in" }
```

Target an element with a selector:

- `on_text = "Sign in"`: the text a user reads. Glob (`*`, `?`), case-insensitive, full string: `"Item *"`. Prefer it.
- `on_accessibility_label = "login"`: the accessibility label or id. Use it only to test that label (screen readers), or when the element has no text.
- `on_below`, `on_above`, `on_right_of`, `on_left_of`: near another element. `on_index` picks one of several matches, from 0.
- A group: `on = { text = "OK", below = "Confirm?" }`. help("selectors") has the rest.

Options on any step: `timeout` (ms), `auto_scroll = true` (scroll to find the element), `if_fail = "warn"` or `"ignore"`, `retry` (count), `save_to` (a variable). help("flow", "step/options") lists them all.

A value can use a variable, `${name}`, or fake data, `${fake:email}` (help("fake")).

The common actions:

```toml
{ action = "tap", on_text = "Sign in" }
{ action = "type", on_text = "Email", input = "${fake:email}" }
{ action = "assert_visible", on_text = "Welcome*" }
{ action = "assert_not_visible", on_text = "Loading" }
{ action = "scroll", to = { text = "Terms" } }
{ action = "read", on_right_of = "Total:", save_to = "total" }
```

help("act", action) gives one action's fields and examples.

Items (help("act", item)):
- interaction: Touch and keyboard input on an element — tap, double_tap, type, backspace, clear_text, long_press, swipe, scroll, pinch, gesture, rotate, hide_keyboard
- assertions: Check what the screen shows; each waits up to its timeout — assert_visible, assert_not_visible, assert_alert
- reading: Read an element's text into a variable — read
- app-lifecycle: Start, stop and reset an app, and set its permissions — launch, app-permissions, stop, clear_data
- device-controls: Change device settings: dark mode, location, hardware buttons — set_dark_mode, set_location, press
- capture: Screenshots, screen recordings, and media files pushed to the device — screenshot, recording, add_media
- alerts: Accept or dismiss a dialog from the app or the OS — accept_alert, dismiss_alert
- external: Links, pushes, shell commands, email, fixtures, mixins and HTTP — open_link, push_notification, bash, run, create_inbox, await_email, load_fixture, load_mixin, get_http, post_http, put_http, patch_http, delete_http
- browser: Drive a host browser, for web state that the app depends on — 20 items: help("act", "browser") lists them; read it first
- flow-control: End the flow on purpose — fail
````

<details><summary><code>help("act", "interaction")</code> · Interaction · 449 chars · + 12 items</summary>

Source: [src/actions-reference/10-interaction/00-intro.md](src/actions-reference/10-interaction/00-intro.md)

```text
## Interaction

Touch and keyboard input on an element.

Items (help("act", item)):
- tap: Tap an element
- double_tap: Double-tap an element
- type: Type text into an element
- backspace: Delete characters
- clear_text: Empty a field
- long_press: Long press an element
- swipe: Swipe gesture
- scroll: Scroll until element found
- pinch: Pinch zoom gesture
- gesture: Multi-touch gesture
- rotate: Rotate gesture
- hide_keyboard: Dismiss keyboard
```

<details><summary><code>help("act", "tap")</code> · tap — Tap an element · 437 chars</summary>

Source: [src/actions-reference/10-interaction/10-tap.md](src/actions-reference/10-interaction/10-tap.md)

````text
### `tap` — Tap an element

Find an element matching the selectors and tap its center.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "tap", on_text = "+", timeout = 5000 }
{ action = "tap", on = { text = "OK", below = "Confirm?" } }
{ action = "tap", on_accessibility_label = "Increment" }
```

On iOS a tap holds for about 50 ms. If the app reads that as a long press,
use `long_press` (help("act", "long_press")) instead.
````

</details>

<details><summary><code>help("act", "double_tap")</code> · double_tap — Double-tap an element · 175 chars</summary>

Source: [src/actions-reference/10-interaction/20-double_tap.md](src/actions-reference/10-interaction/20-double_tap.md)

````text
### `double_tap` — Double-tap an element

Two rapid taps at the element center.

```toml
{ action = "double_tap", on_text = "Zoom" }
```

Same selectors and options as `tap`.
````

</details>

<details><summary><code>help("act", "type")</code> · type — Type text into an element · 489 chars</summary>

Source: [src/actions-reference/10-interaction/30-type.md](src/actions-reference/10-interaction/30-type.md)

````text
### `type` — Type text into an element

With a selector, taps the element to focus it, then types the `input`
string. No selector = type into the focused field.

```toml
{ action = "type", on_text = "Email", input = "user@example.com" }
{ action = "type", on_text = "Search", input = "${query}" }
{ action = "type", input = " and more" }   # append to the focused field
```

| Field | Description |
|-------|-------------|
| `input` | Text to type. Supports `${variable}` interpolation. |
````

</details>

<details><summary><code>help("act", "backspace")</code> · backspace — Delete characters · 486 chars</summary>

Source: [src/actions-reference/10-interaction/40-backspace.md](src/actions-reference/10-interaction/40-backspace.md)

````text
### `backspace` — Delete characters

Deletes `count` characters before the caret in the **currently focused** text
field. `type` or `tap` the field first; `type` leaves the caret at the end of
the text. A selector is an error.

```toml
{ action = "type", on_text = "Email", input = "me@example.comm" },
{ action = "backspace", count = 1 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `count` | `1` | Number of characters to delete from the focused field |
````

</details>

<details><summary><code>help("act", "clear_text")</code> · clear_text — Empty a field · 802 chars</summary>

Source: [src/actions-reference/10-interaction/50-clear_text.md](src/actions-reference/10-interaction/50-clear_text.md)

````text
### `clear_text` — Empty a field

Empties the **currently focused** text field in one step, whatever its length.
Like `backspace` it takes **no selector** — `type` or `tap` the field first.
Use it when the field's contents aren't known up front (a value carried over
from a previous run, a prefilled form); reach for `backspace` when you want to
remove a specific number of characters.

```toml
{ action = "tap", on_text = "Email" },
{ action = "clear_text" },
{ action = "type", input = "me@example.com" }
```

Takes no fields.

- **The caret must be at the end.** If it isn't, the step fails and tells you
  to re-focus the field. `type` leaves the caret at the end; a `tap` places it
  where you tapped.
- **A field whose contents exactly equal its placeholder reads as empty** and is
  left alone.
````

</details>

<details><summary><code>help("act", "long_press")</code> · long_press — Long press an element · 269 chars</summary>

Source: [src/actions-reference/10-interaction/60-long_press.md](src/actions-reference/10-interaction/60-long_press.md)

````text
### `long_press` — Long press an element

Press and hold at the element center.

```toml
{ action = "long_press", on_text = "Item", duration = 2000 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `duration` | `1000` | Hold duration in ms |
````

</details>

<details><summary><code>help("act", "swipe")</code> · swipe — Swipe gesture · 1473 chars</summary>

Source: [src/actions-reference/10-interaction/70-swipe.md](src/actions-reference/10-interaction/70-swipe.md)

````text
### `swipe` — Swipe gesture

`swipe` is the **raw** gesture primitive — one direction-based swipe or a path-based gesture defined by `start` / `end` (and optional `points` for 3+ point paths). Use `scroll` instead when you want golem to *keep* swiping until an element appears.

```toml
# Direction-based — single swipe from a sensible default origin
{ action = "swipe", direction = "down" }
{ action = "swipe", direction = "left" }

# Path-based with selectors — start and end resolve to element centres
{ action = "swipe", start = { text = "Slider" }, end = { text = "Max" } }

# Anchored to a container: end is 30% of the element's height below its centre
{ action = "swipe",
  start = { below = "Scroll List" },
  end   = { below = "Scroll List", y = "30%" } }
```

| Field | Description |
|-------|-------------|
| `direction` | `"up"`, `"down"`, `"left"`, `"right"` |
| `start` | Start position: a selector group (`text` / `accessibility_label` / `below` / `above`) plus optional `x` / `y`. With an element, `x` / `y` offset from its centre: pixels, or `"N%"` of the element's size (`"50%"` = edge). Without an element, `x` / `y` are screen pixels or `"N%"` of the screen. |
| `end` | End position, same format as `start` |
| `points` | Array of intermediate points, same format as `start` |
| `duration` | Gesture duration in ms for a path of 3+ points (default `300`); a 2-point swipe ignores it |

`within` is ignored on swipe (lint warning); use `start` / `end`.
````

</details>

<details><summary><code>help("act", "scroll")</code> · scroll — Scroll until element found · 818 chars</summary>

Source: [src/actions-reference/10-interaction/80-scroll.md](src/actions-reference/10-interaction/80-scroll.md)

````text
### `scroll` — Scroll until element found

Scrolls the page (or a container) until the target element is visible.

```toml
# Scroll page to find element
{ action = "scroll", to = { text = "Item 25" }, timeout = 60000 }

# Scroll within a specific container
{ action = "scroll", to = { text = "Item 45" }, within = { below = "Scroll List" }, timeout = 60000 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `to` | — | Target element: a selector group (alias of `on`), or use the flat `on_*` selectors |
| `direction` | `"down"` | Scroll direction |
| `within` | — | Constrain scrolling to an element's bounds |
| `scroll_timeout` | — | Stop scrolling after this many ms, within the step `timeout` |
| `timeout` | 8× `step_timeout` (40 s); 12× (60 s) with `within` | Overall scroll timeout |
````

</details>

<details><summary><code>help("act", "pinch")</code> · pinch — Pinch zoom gesture · 750 chars</summary>

Source: [src/actions-reference/10-interaction/90-pinch.md](src/actions-reference/10-interaction/90-pinch.md)

````text
### `pinch` — Pinch zoom gesture

Two-finger pinch centered on an element or coordinates.

```toml
{ action = "pinch", scale = 2.0 }                     # Zoom in
{ action = "pinch", scale = 0.5, velocity = 2.0 }     # Zoom out, slowly
{ action = "pinch", on_text = "Map", scale = 2.0 }    # Centered on an element
{ action = "pinch", x = "50%", y = 300, scale = 0.5 } # Centered on a point
```

| Field | Default | Description |
|-------|---------|-------------|
| `scale` | — | `>1.0` = zoom in, `<1.0` = zoom out |
| `velocity` | `5.0` | Scale factor per second |
| center | screen centre | An element (`on_text`, `on_accessibility_label` or `on = { … }`), or `x` / `y` as pixels or `"N%"` of the screen. With an element, `x` / `y` are ignored. |
````

</details>

<details><summary><code>help("act", "gesture")</code> · gesture — Multi-touch gesture · 806 chars</summary>

Source: [src/actions-reference/10-interaction/100-gesture.md](src/actions-reference/10-interaction/100-gesture.md)

````text
### `gesture` — Multi-touch gesture

Arbitrary multi-finger gesture with explicit paths.

```toml
[[block.steps]]
action = "gesture"
duration = 300

[[block.steps.fingers]]
points = [
  { x = 200, y = 400 },
  { x = 200, y = 200 },
]

[[block.steps.fingers]]
points = [
  { x = 200, y = 200 },
  { x = 200, y = 400 },
]
```

| Field | Default | Description |
|-------|---------|-------------|
| `fingers` | — | Array of finger paths, each with `points` (at least 2 per finger) |
| `duration` | `300` | Time (ms) each finger takes to travel its whole path |

A point is `x` / `y` screen coordinates (pixels or `"N%"` of the screen), or a
selector group (`text` / `accessibility_label` / `below` / `above`) plus
optional `x` / `y` offsets from the element's centre (pixels, or `"N%"` of the
element's size).
````

</details>

<details><summary><code>help("act", "rotate")</code> · rotate — Rotate gesture · 860 chars</summary>

Source: [src/actions-reference/10-interaction/110-rotate.md](src/actions-reference/10-interaction/110-rotate.md)

````text
### `rotate` — Rotate gesture

A two-finger **rotation gesture** centered on an element (or screen). `rotate` is a multi-touch gesture, **not** a device-orientation change — programmatic device orientation is unsupported.

Two fingers orbit a center point: an element (`on_text`, `on_accessibility_label` or `on = { … }`), or `x` / `y` as pixels or `"N%"` of the screen. Without either, the screen centre. With an element, `x` / `y` are ignored.

```toml
{ action = "rotate", on_text = "Map", rotation = 90.0 }    # rotate 90° clockwise
{ action = "rotate", on_text = "Map", rotation = -45.0 }   # 45° counter-clockwise
```

| Field | Default | Description |
|-------|---------|-------------|
| `rotation` | — (required) | Degrees to rotate. Positive = clockwise, negative = counter-clockwise. |
| `velocity` | `180.0` | Rotation speed in degrees per second |
````

</details>

<details><summary><code>help("act", "hide_keyboard")</code> · hide_keyboard — Dismiss keyboard · 463 chars</summary>

Source: [src/actions-reference/10-interaction/120-hide_keyboard.md](src/actions-reference/10-interaction/120-hide_keyboard.md)

````text
### `hide_keyboard` — Dismiss keyboard

Dismiss the on-screen keyboard. No-op if no keyboard is visible.

```toml
{ action = "hide_keyboard" }
```

You rarely need it to reach a field the keyboard covers: when the keyboard
hides a step's target, golem dismisses the keyboard and looks again. Set
`keep_keyboard = true` on a step to opt out (see
Step Options (help("flow", "step/options"))):

```toml
{ action = "tap", on_text = "Done", keep_keyboard = true }
```
````

</details>

</details>

<details><summary><code>help("act", "assertions")</code> · Assertions · 282 chars · + 3 items</summary>

Source: [src/actions-reference/20-assertions/00-intro.md](src/actions-reference/20-assertions/00-intro.md)

```text
## Assertions

Check what the screen shows. An assertion waits up to its timeout for the screen to match.

Items (help("act", item)):
- assert_visible: Wait for / assert element exists
- assert_not_visible: Wait for / assert element absent
- assert_alert: Assert alert is displayed
```

<details><summary><code>help("act", "assert_visible")</code> · assert_visible — Wait for / assert element exists · 702 chars</summary>

Source: [src/actions-reference/20-assertions/10-assert_visible.md](src/actions-reference/20-assertions/10-assert_visible.md)

````text
### `assert_visible` — Wait for / assert element exists

Poll the hierarchy until an element matching the selectors is on screen, or `timeout` elapses (default 10s). Use a short `timeout` for instantaneous checks, a long one for waits. The assertion is driven by the selectors — add `on_enabled` / `on_checked` to assert state, not just presence.

```toml
{ action = "assert_visible", on_text = "Welcome" }
{ action = "assert_visible", on_text = "1", on_below = "Counter" }
{ action = "assert_visible", on_text = "Submit", on_enabled = true }                    # state, not just presence
{ action = "assert_visible", on_text = "Item 0", auto_scroll = true, timeout = 60000 }  # off-screen element
```
````

</details>

<details><summary><code>help("act", "assert_not_visible")</code> · assert_not_visible — Wait for / assert element absent · 433 chars</summary>

Source: [src/actions-reference/20-assertions/20-assert_not_visible.md](src/actions-reference/20-assertions/20-assert_not_visible.md)

````text
### `assert_not_visible` — Wait for / assert element absent

Poll until no element matching the selectors is visible on screen, or `timeout` elapses (default 10s). An element still in the hierarchy but scrolled off screen, clipped by its container, or behind the keyboard counts as not visible.

```toml
{ action = "assert_not_visible", on_text = "Error" }
{ action = "assert_not_visible", on_text = "Loading", timeout = 10000 }
```
````

</details>

<details><summary><code>help("act", "assert_alert")</code> · assert_alert — Assert alert is displayed · 228 chars</summary>

Source: [src/actions-reference/20-assertions/30-assert_alert.md](src/actions-reference/20-assertions/30-assert_alert.md)

````text
### `assert_alert` — Assert alert is displayed

Verify an alert/dialog is showing. Optionally match alert text with a glob pattern.

```toml
{ action = "assert_alert" }
{ action = "assert_alert", on_text = "Are you sure*" }
```
````

</details>

</details>

<details><summary><code>help("act", "reading")</code> · Reading · 106 chars · + 1 item</summary>

Source: [src/actions-reference/30-reading/00-intro.md](src/actions-reference/30-reading/00-intro.md)

```text
## Reading

Read an element's text into a variable.

Items (help("act", item)):
- read: Read element text
```

<details><summary><code>help("act", "read")</code> · read — Read element text · 342 chars</summary>

Source: [src/actions-reference/30-reading/10-read.md](src/actions-reference/30-reading/10-read.md)

````text
### `read` — Read element text

Find an element and capture its text into a variable.

```toml
{ action = "read", on_right_of = "Status:", save_to = "status" }
{ action = "read", on_below = "Counter", on_index = 0, save_to = "count" }
```

| Field | Description |
|-------|-------------|
| `save_to` | Variable name to store the text value |
````

</details>

</details>

<details><summary><code>help("act", "app-lifecycle")</code> · App Lifecycle · 273 chars · + 4 items</summary>

Source: [src/actions-reference/40-app-lifecycle/00-intro.md](src/actions-reference/40-app-lifecycle/00-intro.md)

```text
## App Lifecycle

Start, stop and reset an app, and set its permissions.

Items (help("act", item)):
- launch: Launch or foreground an app
- app-permissions: Grant or deny camera, photos, location and other permissions
- stop: Terminate an app
- clear_data: Clear app data
```

<details><summary><code>help("act", "launch")</code> · launch — Launch or foreground an app · 898 chars</summary>

Source: [src/actions-reference/40-app-lifecycle/10-launch.md](src/actions-reference/40-app-lifecycle/10-launch.md)

````text
### `launch` — Launch or foreground an app

Bring an app to the foreground. Does not restart if already running. Use `restart = true` for a cold start.

```toml
{ action = "launch", app = "app" }
{ action = "launch", app = "companion" }
{ action = "launch", app = "app", restart = true }                        # Kill and relaunch
{ action = "launch", app = "app", permissions = { camera = "allow" } }    # Grant, then cold start
```

| Field | Default | Description |
|-------|---------|-------------|
| `app` | — | App name (as defined in `[[flow.apps]]`) |
| `restart` | `false` | Stop app first, then launch fresh |
| `permissions` | — | Per-launch permission map (see App permissions (help("act", "app-permissions"))) |

A `permissions` map always cold-starts the app, even without `restart = true`. Use it to flip a permission mid-flow and test both the granted and denied paths in one flow.
````

</details>

<details><summary><code>help("act", "app-permissions")</code> · App permissions · 1620 chars</summary>

Source: [src/actions-reference/40-app-lifecycle/20-app-permissions.llm.md](src/actions-reference/40-app-lifecycle/20-app-permissions.llm.md) (LLM text)

````text
### App permissions

Set permissions with a `permission = mode` map, in `[[flow.apps]].permissions` (applied before the first launch) or on a `launch` step (always cold-starts the app). There is no standalone permission step.

```toml
{ action = "launch", app = "app", permissions = { camera = "allow" } }
{ action = "launch", app = "app", permissions = { camera = "deny", location = "always" } }
```

| Mode | Applies to | Meaning |
|------|-----------|---------|
| `allow` / `deny` | any permission | grant / explicitly denied. `location = "allow"` is foreground only. |
| `always` | `location` only | background + foreground location |
| `limited` | `photos` only | partial photo-library access |

Any other mode/permission pairing is a parse error.

Permission names: `camera`, `microphone`, `location`, `contacts`, `calendar`, `photos`. Unknown names are an error. Android also accepts a full `android.permission.*` string.

Android: the app's `AndroidManifest.xml` must declare every permission you grant (`photos` needs `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED` and `READ_EXTERNAL_STORAGE`; `location = "always"` also needs `ACCESS_BACKGROUND_LOCATION`).

iOS `photos` grants without a prompt only when `applesimutils` is installed. Without it, the app prompts at runtime; add this after the step that triggers photo access:

```toml
{ action = "accept_alert", if_fail = "ignore" }
```

Notifications can't be pre-granted. Trigger the request in the app, then accept the system dialog:

```toml
{ action = "tap", on_text = "Enable Notifications" }
{ action = "accept_alert", if_fail = "ignore" }
```
````

</details>

<details><summary><code>help("act", "stop")</code> · stop — Terminate an app · 141 chars</summary>

Source: [src/actions-reference/40-app-lifecycle/30-stop.md](src/actions-reference/40-app-lifecycle/30-stop.md)

````text
### `stop` — Terminate an app

Terminate the app's process. The next `launch` starts it fresh.

```toml
{ action = "stop", app = "app" }
```
````

</details>

<details><summary><code>help("act", "clear_data")</code> · clear_data — Clear app data · 652 chars</summary>

Source: [src/actions-reference/40-app-lifecycle/40-clear_data.md](src/actions-reference/40-app-lifecycle/40-clear_data.md)

````text
### `clear_data` — Clear app data

Clear the app's storage and cache.

```toml
{ action = "clear_data", app = "app" }
```

iOS: simulator only; gate on `_hardware`. Android works on emulators and physical devices.

To reset state on a physical iOS device, either drive the app's own "sign out" / "reset" affordance, or reinstall it (the install script runs before every flow; `GOLEM_REBUILD` forces a fresh build). Gate the step on device class if one flow must cover both:

```toml
[[block.branch]]
if_var = "_hardware"
equals = "virtual"
goto = "wipe_via_clear_data"
[[block.branch]]
if_var = "_hardware"
equals = "real"
goto = "wipe_via_app_ui"
```
````

</details>

</details>

<details><summary><code>help("act", "device-controls")</code> · Device Controls · 194 chars · + 3 items</summary>

Source: [src/actions-reference/50-device-controls/00-intro.md](src/actions-reference/50-device-controls/00-intro.md)

```text
## Device Controls

Change device settings: dark mode, location, hardware buttons.

Items (help("act", item)):
- set_dark_mode
- set_location: Set GPS coordinates
- press: Press hardware button
```

<details><summary><code>help("act", "set_dark_mode")</code> · set_dark_mode — Set dark mode · 341 chars</summary>

Source: [src/actions-reference/50-device-controls/10-set_dark_mode.md](src/actions-reference/50-device-controls/10-set_dark_mode.md)

````text
### `set_dark_mode` — Set dark mode

Switch the device's system appearance to dark (`enabled = true`) or light (`enabled = false`).

```toml
{ action = "set_dark_mode", enabled = true }
{ action = "set_dark_mode", enabled = false }
```

iOS: simulator only; gate on `_hardware` (see `clear_data` (help("act", "clear_data")) for the branch).
````

</details>

<details><summary><code>help("act", "set_location")</code> · set_location — Set GPS coordinates · 308 chars</summary>

Source: [src/actions-reference/50-device-controls/20-set_location.md](src/actions-reference/50-device-controls/20-set_location.md)

````text
### `set_location` — Set GPS coordinates

Set the device's GPS position to `latitude` / `longitude` (decimal degrees).

```toml
{ action = "set_location", latitude = 37.7749, longitude = -122.4194 }
```

iOS: simulator only; gate on `_hardware` (see `clear_data` (help("act", "clear_data")) for the branch).
````

</details>

<details><summary><code>help("act", "press")</code> · press — Press hardware button · 477 chars</summary>

Source: [src/actions-reference/50-device-controls/30-press.md](src/actions-reference/50-device-controls/30-press.md)

````text
### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

**Supported buttons (platform-specific):**

| `button`      | Android | iOS |
|---------------|---------|-----|
| `home`        | ✓       | ✓   |
| `back`        | ✓       | —   |
| `volume_up`   | ✓       | —   |
| `volume_down` | ✓       | —   |

An unsupported button fails the step.
````

</details>

</details>

<details><summary><code>help("act", "capture")</code> · Capture · 215 chars · + 3 items</summary>

Source: [src/actions-reference/60-capture/00-intro.md](src/actions-reference/60-capture/00-intro.md)

```text
## Capture

Screenshots, screen recordings, and media files pushed to the device.

Items (help("act", item)):
- screenshot: Take screenshot
- recording: per-block via record = true
- add_media: Push media to device
```

<details><summary><code>help("act", "screenshot")</code> · screenshot — Take screenshot · 296 chars</summary>

Source: [src/actions-reference/60-capture/10-screenshot.md](src/actions-reference/60-capture/10-screenshot.md)

````text
### `screenshot` — Take screenshot

Capture the screen. With `path`, save it there; a relative path resolves from the directory where you run golem. Without `path`, the image is captured but not saved.

```toml
{ action = "screenshot" }
{ action = "screenshot", path = "/tmp/dark-mode.png" }
```
````

</details>

<details><summary><code>help("act", "recording")</code> · Screen recording — per-block via record = true · 545 chars</summary>

Source: [src/actions-reference/60-capture/20-recording.md](src/actions-reference/60-capture/20-recording.md)

````text
### Screen recording — per-block via `record = true`

Recording is configured with `record`, not as a step action (see Flow Options (help("flow", "flow/options"))). Highest priority wins: `--no-record` > `--record` > `[[block]] record` > `[flow.options] record` > `[options] record`. Output: `{output_dir}/{flow}/{device}/recordings/{block}_{iter}.mp4`.

```toml
[[block]]
name = "login"
record = true     # record this block only
steps = [ ... ]
```

iOS: simulator only. On a physical iPhone the block logs a warning and runs without a video.
````

</details>

<details><summary><code>help("act", "add_media")</code> · add_media — Push media to device · 535 chars</summary>

Source: [src/actions-reference/60-capture/30-add_media.md](src/actions-reference/60-capture/30-add_media.md)

````text
### `add_media` — Push media to device

Add an image or video file to the device's photo library. golem checks the file's contents: anything other than an image (png, jpeg, gif, webp, heif, bmp, tiff) or a video (mp4, mov) fails with P462. A relative `path` resolves from the directory where you run golem.

```toml
{ action = "add_media", path = "fixtures/photo.jpg" }
```

iOS: simulator only; gate on `_hardware` (see `clear_data` (help("act", "clear_data")) for the branch), or put the file in the device's library before the run.
````

</details>

</details>

<details><summary><code>help("act", "alerts")</code> · Alerts · 152 chars · + 2 items</summary>

Source: [src/actions-reference/70-alerts/00-intro.md](src/actions-reference/70-alerts/00-intro.md)

```text
## Alerts

Accept or dismiss a dialog from the app or the OS.

Items (help("act", item)):
- accept_alert: Accept dialog
- dismiss_alert: Dismiss dialog
```

<details><summary><code>help("act", "accept_alert")</code> · accept_alert — Accept dialog · 411 chars</summary>

Source: [src/actions-reference/70-alerts/10-accept_alert.md](src/actions-reference/70-alerts/10-accept_alert.md)

````text
### `accept_alert` — Accept dialog

Tap the positive button (OK, Yes, Allow) on the current alert. It also handles OS prompts, such as permission requests and "Open in …?" dialogs.

```toml
{ action = "accept_alert" }
{ action = "accept_alert", if_fail = "ignore" }   # prompt may not appear
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a prompt that may not appear.
````

</details>

<details><summary><code>help("act", "dismiss_alert")</code> · dismiss_alert — Dismiss dialog · 362 chars</summary>

Source: [src/actions-reference/70-alerts/20-dismiss_alert.md](src/actions-reference/70-alerts/20-dismiss_alert.md)

````text
### `dismiss_alert` — Dismiss dialog

Tap the negative button (Cancel, No) on the current in-app alert. It may not reach an OS prompt (an iOS permission dialog, for example); `accept_alert` handles those.

```toml
{ action = "dismiss_alert" }
```

The step fails if no alert appears before the timeout. Use `if_fail = "ignore"` for a dialog that may not appear.
````

</details>

</details>

<details><summary><code>help("act", "external")</code> · External · 435 chars · + 9 items</summary>

Source: [src/actions-reference/80-external/00-intro.md](src/actions-reference/80-external/00-intro.md)

```text
## External

Links, pushes, shell commands, email, fixtures, mixins and HTTP.

Items (help("act", item)):
- open_link: Open URL or deep link
- push_notification: Deliver a push to the app under test
- bash: Run shell command
- run: Run project script
- create_inbox: Provision a disposable email inbox
- await_email: Poll IMAP inbox
- load_fixture: Load fixture data
- load_mixin: Inline a reusable step sequence
- http: HTTP requests
```

<details><summary><code>help("act", "open_link")</code> · open_link — Open URL or deep link · 201 chars</summary>

Source: [src/actions-reference/80-external/10-open_link.md](src/actions-reference/80-external/10-open_link.md)

````text
### `open_link` — Open URL or deep link

Open a URL or deep link on the device.

```toml
{ action = "open_link", url = "https://example.com" }
{ action = "open_link", url = "myapp://profile/123" }
```
````

</details>

<details><summary><code>help("act", "push_notification")</code> · push_notification — Deliver a push to the app under test · 948 chars</summary>

Source: [src/actions-reference/80-external/20-push_notification.llm.md](src/actions-reference/80-external/20-push_notification.llm.md) (LLM text)

````text
### `push_notification` — Deliver a push to the app under test

Deliver a push notification to the app under test, without APNs or FCM.

```toml
{ action = "push_notification", title = "New message", body = "Hello!" }
```

| Field | Description |
|-------|-------------|
| `title` | Notification title |
| `body` | Notification body |
| `payload` | Optional. iOS only: a string holding a JSON object, added to the APNs payload as `custom`. Ignored on Android. |

Simulator and emulator only, on both platforms. Gate the step on `_hardware` when the flow may run on real hardware:

```toml
[[block.branch]]
if_var = "_hardware"
equals = "virtual"
goto = "send_push"
```

Set `hardware = "virtual"` on the app to silence the real-hardware lint warning.

The app needs its own receive bridge: a `UNUserNotificationCenterDelegate` on iOS; on Android a `BroadcastReceiver` for `<package>.PUSH_NOTIFICATION` reading the `title` and `body` string extras.
````

</details>

<details><summary><code>help("act", "bash")</code> · bash — Run shell command · 310 chars</summary>

Source: [src/actions-reference/80-external/30-bash.md](src/actions-reference/80-external/30-bash.md)

````text
### `bash` — Run shell command

Execute a command via `sh -c`. A non-zero exit code fails the step and reports the command's stderr. `save_to` stores stdout, trimmed.

```toml
{ action = "bash", run = "curl -s https://api.example.com/reset" }
{ action = "bash", run = "echo $ENV_VAR", save_to = "result" }
```
````

</details>

<details><summary><code>help("act", "run")</code> · run — Run project script · 635 chars</summary>

Source: [src/actions-reference/80-external/40-run.md](src/actions-reference/80-external/40-run.md)

````text
### `run` — Run project script

Execute a script relative to the project root or flow directory. Rejects path traversal (`..`).

```toml
{ action = "run", script = "/scripts/seed_db.sh" }
{ action = "run", script = "/scripts/setup.sh", args = ["staging", "verbose"], save_to = "output" }
```

Leading `/` = relative to project root. No leading `/` = relative to flow file directory.

The script runs directly, not through a shell, so it must be executable (`chmod +x`) and start with a shebang. A non-zero exit code fails the step and reports stderr. `save_to` stores an object: `${output.stdout}` (trimmed) and `${output.exit_code}`.
````

</details>

<details><summary><code>help("act", "create_inbox")</code> · create_inbox — Provision a disposable email inbox · 1410 chars</summary>

Source: [src/actions-reference/80-external/50-create_inbox.md](src/actions-reference/80-external/50-create_inbox.md)

````text
### `create_inbox` — Provision a disposable email inbox

Provision a fresh inbox from a provider and save its connection details as an
object for later steps. The saved object's `imap_host`/`imap_port`/`user`/`pass`
fields are exactly what `await_email` (help("act", "await_email")) reads, so
`save_to = "inbox"` feeds straight into `await_email { inbox = "inbox" }`.

```toml
{ action = "create_inbox", provider = "ethereal", save_to = "inbox" }
{ action = "type", on_text = "Email", input = "${inbox.address}" }
# … app signup …
{ action = "await_email", inbox = "inbox", subject = "*verify*", extract = { otp = "code: (\\d{6})" }, save_to = "mail" }
{ action = "type", input = "${mail.otp}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `provider` | — | Inbox provider. Only `ethereal` is built in; any other value errors. |
| `save_to` | — | Variable to store the inbox object under (required). |
| `timeout` | `15000` | Provisioning deadline (ms). |

Saved object fields: `address` (= `user`, the email address), `user`, `pass`,
`imap_host`, `imap_port`, `smtp_host`, `smtp_port`. `pass` is always
secret (help("flow", "secrets")): golem masks it in all output.

> **Non-deterministic.** Provisioning is live network I/O, so the inbox is not
> replayed by `--seed` — each run gets a fresh address. Receiving mail at it is
> live too (`await_email` connects over real IMAP).
````

</details>

<details><summary><code>help("act", "await_email")</code> · await_email — Poll IMAP inbox · 1314 chars</summary>

Source: [src/actions-reference/80-external/60-await_email.md](src/actions-reference/80-external/60-await_email.md)

````text
### `await_email` — Poll IMAP inbox

Poll an inbox over IMAP (TLS) and wait for an email matching the filters, with
optional regex extraction.

`inbox` is the **name of a variable** holding an inbox object, not the email
address. See `create_inbox` (help("act", "create_inbox"))
for how the two pair up.

```toml
# Pairs with create_inbox { save_to = "inbox" }:
{ action = "await_email", inbox = "inbox", subject = "Verify*", timeout = 30000, save_to = "email" }

# …or a hand-written inbox object:
# [flow.vars]
# inbox = { imap_host = "imap.example.com", imap_port = "993", user = "me@example.com", pass = "secret" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `inbox` | — | Name of a variable holding an inbox object; the `imap_host` / `imap_port` / `user` / `pass` fields on it are used to connect |
| `recipient` | — | Glob filter for the recipient address (not `to`) |
| `subject` | `"*"` | Subject glob pattern |
| `extract` | — | Table of field names to regex patterns |
| `timeout` | `30000` | Polling timeout (ms) |

When more than one email matches, the **most recent** is returned, so a stale
match left in the inbox from an earlier run never shadows the fresh one. Only
the latest messages are scanned (not the entire mailbox), which is ample for
verification/OTP mail.
````

</details>

<details><summary><code>help("act", "load_fixture")</code> · load_fixture — Load fixture data · 882 chars</summary>

Source: [src/actions-reference/80-external/70-load_fixture.md](src/actions-reference/80-external/70-load_fixture.md)

````text
### `load_fixture` — Load fixture data

Load variables from a TOML file in `__fixtures__/` (a `[vars]` table). See
reuse comparison (help("flow", "reuse")).

```toml
{ action = "load_fixture", fixture = "users", as = "test_user" }
# Access as ${test_user.email}, ${test_user.name}, etc.
```

| Field | Description |
|-------|-------------|
| `fixture` | Fixture name: `__fixtures__/<fixture>.toml`, looked up from the flow's directory up to the project root (required) |
| `as` | Variable name the fixture's vars are stored under (required) |

A root `secret` list names the keys of `[vars]` that golem masks in all output. A name that is not a key of `[vars]` fails the fixture, so a typo cannot leave a value unmasked. See Secrets (help("flow", "secrets")).

```toml
# __fixtures__/qa.toml
secret = ["password"]

[vars]
email = "qa@example.test"
password = "staging-password"
```
````

</details>

<details><summary><code>help("act", "load_mixin")</code> · load_mixin — Inline a reusable step sequence · 819 chars</summary>

Source: [src/actions-reference/80-external/80-load_mixin.md](src/actions-reference/80-external/80-load_mixin.md)

````text
### `load_mixin` — Inline a reusable step sequence

Inline the steps from a mixin file in `__mixins__/` (a `[[step]]`-only file) into
the current block or `[[teardown]]`. In an MCP session, `act` runs the mixin's
steps, and the draft records the `load_mixin` step. Pass per-call values via `vars`, referenced as `${…}` inside
the mixin. See reuse comparison (help("flow", "reuse")).

```toml
# __mixins__/launch_and_wait.toml
[[step]]
action = "launch"
app = "${app_bundle}"
[[step]]
action = "assert_visible"
on_text = "${wait_element}"
```

```toml
{ action = "load_mixin", mixin = "launch_and_wait", vars = { app_bundle = "app", wait_element = "Submit" } }
```

A mixin is a step fragment that runs inside the caller's block; for reusing a
whole scenario as a child, use a subflow (help("flow", "subflow")) instead.
````

</details>

<details><summary><code>help("act", "http")</code> · get_http, post_http, put_http, patch_http, delete_http — HTTP requests · 668 chars</summary>

Source: [src/actions-reference/80-external/90-http.md](src/actions-reference/80-external/90-http.md)

````text
### `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http` — HTTP requests

```toml
{ action = "get_http", url = "https://api.example.com/status", save_to = "response" }
{ action = "post_http", url = "https://api.example.com/reset", body = "{\"force\": true}" }
{ action = "get_http", url = "https://api.example.com/data", headers = { Authorization = "Bearer ${token}" } }
```

| Field | Description |
|-------|-------------|
| `url` | Request URL (required) |
| `body` | Request body, as a string |
| `headers` | Table of header name to string value |
| `save_to` | Variable to store the response body under, as a string |

A non-2xx status fails the step.
````

</details>

</details>

<details><summary><code>help("act", "browser")</code> · Browser · 2227 chars · + 20 items</summary>

Source: [src/actions-reference/90-browser/00-intro.llm.md](src/actions-reference/90-browser/00-intro.llm.md) (LLM text)

````text
## Browser

Drive a host browser, for web state that the app depends on.

`browse_*` steps drive a host Chrome/Chromium (or `$CHROME`); none found → plan fails with `H424`. Each flow gets its own browser, closed when the flow ends.

Targeting is **CSS only** (`selector`); mobile selectors (`text`, `on_below`, …) are ignored. Steps check DOM presence, not visibility: an element hidden by CSS counts as present.

| Field | Default | Description |
|-------|---------|-------------|
| `selector` | — | CSS selector. Required by `browse_tap`, `browse_type`, `browse_read`, `browse_select`, `browse_scroll_to`, `browse_assert_*`, `browse_wait_*` |
| `index` | `0` | Which match to act on, 0-based |
| `session` | `_default` | `[context:]tab` |
| `timeout` | `5000`; `10000` for `browse_wait_*` | ms to keep looking for the element |

Tabs in one context share cookies; separate contexts share nothing. `session = "admin"` is a tab in the default context; `session = "tenantB:admin"` is tab `admin` in context `tenantB`. Labels may not contain `:`.

```toml
{ action = "browse_navigate", url = "${portal}", session = "tenantA:main" }
{ action = "browse_navigate", url = "${portal}", session = "tenantB:main" }
```

Storage and cookie steps need a real origin (a page loaded by `browse_navigate`); they fail on `about:blank`.

Items (help("act", item)):
- browse_navigate: Load a URL
- browse_tap: Click an element
- browse_type: Type into a field
- browse_read: Read text or an attribute into a variable
- browse_screenshot: Capture the tab
- browse_assert_exists: The element is in the DOM
- browse_assert_not_exists: The element is not in the DOM
- browse_assert_text: The element says what you expect
- browse_wait_exists: Wait for an element to appear
- browse_wait_not_exists: Wait for an element to disappear
- browse_scroll_by: Scroll by a distance
- browse_scroll_to: Bring an element into view
- browse_select: Choose an option in a <select>
- browse_execute_js: Run JavaScript in the page
- browse_mcp_list_tools: List the page's WebMCP tools
- browse_mcp_call: Call a WebMCP tool
- browse_cookies: Cookies
- browse_local_storage: Local storage
- browse_session_storage: Session storage
- browse_close: Close a tab early
````

<details><summary><code>help("act", "browse_navigate")</code> · browse_navigate — Load a URL · 805 chars</summary>

Source: [src/actions-reference/90-browser/10-browse_navigate.llm.md](src/actions-reference/90-browser/10-browse_navigate.llm.md) (LLM text)

````text
### `browse_navigate` — Load a URL

```toml
{ action = "browse_navigate", url = "https://portal.example.com/orders" }
{ action = "browse_navigate", url = "${portal}", session = "phone", user_agent = "Mozilla/5.0 (iPhone; …)" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `url` | — | Required |
| `wait_until` | `"domcontentloaded"` | `none`, `domcontentloaded`, or `load` (sub-resources finished too) |
| `user_agent` | the browser's own | User agent for this session, applied before the request |
| `accept_language` | the browser's own | `Accept-Language` for this session, e.g. `"fr-FR"` |

`user_agent` and `accept_language` persist for the session: later steps in that `session` keep them. They do not change layout (the viewport does) or Client Hints (`Sec-CH-UA-*`).
````

</details>

<details><summary><code>help("act", "browse_tap")</code> · browse_tap — Click an element · 209 chars</summary>

Source: [src/actions-reference/90-browser/20-browse_tap.md](src/actions-reference/90-browser/20-browse_tap.md)

````text
### `browse_tap` — Click an element

```toml
{ action = "browse_tap", selector = "#submit" }
{ action = "browse_tap", selector = "button.fulfil", index = 1 }
```

A real click, so the page's own handlers run.
````

</details>

<details><summary><code>help("act", "browse_type")</code> · browse_type — Type into a field · 400 chars</summary>

Source: [src/actions-reference/90-browser/30-browse_type.md](src/actions-reference/90-browser/30-browse_type.md)

````text
### `browse_type` — Type into a field

```toml
{ action = "browse_type", selector = "#email", text = "${inbox.address}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | — | The value to type. `input` is accepted as a synonym, matching the mobile `type` action |

The field is clicked first, so the keystrokes land in it rather than wherever
focus happened to be.
````

</details>

<details><summary><code>help("act", "browse_read")</code> · browse_read — Read text or an attribute into a variable · 590 chars</summary>

Source: [src/actions-reference/90-browser/40-browse_read.md](src/actions-reference/90-browser/40-browse_read.md)

````text
### `browse_read` — Read text or an attribute into a variable

```toml
{ action = "browse_read", selector = "#order-total", save_to = "total" }
{ action = "browse_read", selector = "#order-total", attribute = "data-total", save_to = "total_raw" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `attribute` | — | Read this attribute instead of the element's text, e.g. `data-total="1499"` where the text says `£14.99` |
| `save_to` | — | Variable to save the value in |

Fails with `F404` if the element has no such attribute, rather than saving an empty string.
````

</details>

<details><summary><code>help("act", "browse_screenshot")</code> · browse_screenshot — Capture the tab · 310 chars</summary>

Source: [src/actions-reference/90-browser/50-browse_screenshot.md](src/actions-reference/90-browser/50-browse_screenshot.md)

````text
### `browse_screenshot` — Capture the tab

```toml
{ action = "browse_screenshot", path = "portal-state.png" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `path` | — | Where to write the PNG. Omitted, the capture is taken and discarded — same as the mobile `screenshot` action |
````

</details>

<details><summary><code>help("act", "browse_assert_exists")</code> · browse_assert_exists — The element is in the DOM · 239 chars</summary>

Source: [src/actions-reference/90-browser/60-browse_assert_exists.md](src/actions-reference/90-browser/60-browse_assert_exists.md)

````text
### `browse_assert_exists` — The element is in the DOM

```toml
{ action = "browse_assert_exists", selector = ".order-row[data-state='fulfilled']" }
```

Waits up to `timeout` for the element to appear. Fails with `F404` if it never does.
````

</details>

<details><summary><code>help("act", "browse_assert_not_exists")</code> · browse_assert_not_exists — The element is not in the DOM · 287 chars</summary>

Source: [src/actions-reference/90-browser/70-browse_assert_not_exists.md](src/actions-reference/90-browser/70-browse_assert_not_exists.md)

````text
### `browse_assert_not_exists` — The element is not in the DOM

```toml
{ action = "browse_assert_not_exists", selector = "#error-banner" }
```

Checks once and fails with `F409` if the element is there. It does **not** wait
for something to disappear — that's `browse_wait_not_exists`.
````

</details>

<details><summary><code>help("act", "browse_assert_text")</code> · browse_assert_text — The element says what you expect · 795 chars</summary>

Source: [src/actions-reference/90-browser/80-browse_assert_text.md](src/actions-reference/90-browser/80-browse_assert_text.md)

````text
### `browse_assert_text` — The element says what you expect

```toml
{ action = "browse_assert_text", selector = "#status", text = "Fulfilled" }
{ action = "browse_assert_text", selector = "#total", text = "Total: *" }
{ action = "browse_assert_text", selector = "#total", attribute = "data-total", text = "1499" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `text` | — | Expected value. Supports the same `*` / `?` glob matching (help("selectors")) as mobile text matchers, and is case-sensitive |
| `attribute` | — | Assert on this attribute instead of the element's text |

Polls until the text matches or `timeout` runs out, so a page that updates after
a click isn't judged on what it said beforehand. The failure (`F412`) quotes what
the page actually said.
````

</details>

<details><summary><code>help("act", "browse_wait_exists")</code> · browse_wait_exists — Wait for an element to appear · 331 chars</summary>

Source: [src/actions-reference/90-browser/90-browse_wait_exists.md](src/actions-reference/90-browser/90-browse_wait_exists.md)

````text
### `browse_wait_exists` — Wait for an element to appear

```toml
{ action = "browse_wait_exists", selector = ".order-row" }
{ action = "browse_wait_exists", selector = "#receipt", timeout = 30000 }
```

Polls until the element is in the DOM. Default `timeout` is 10000ms. Running out
fails with `F408` (step timeout), not `F404`.
````

</details>

<details><summary><code>help("act", "browse_wait_not_exists")</code> · browse_wait_not_exists — Wait for an element to disappear · 317 chars</summary>

Source: [src/actions-reference/90-browser/100-browse_wait_not_exists.md](src/actions-reference/90-browser/100-browse_wait_not_exists.md)

````text
### `browse_wait_not_exists` — Wait for an element to disappear

```toml
{ action = "browse_wait_not_exists", selector = ".spinner" }
```

Polls until the element is gone from the DOM. Default `timeout` is 10000ms.
Running out fails with `F408`. To check absence once without waiting, use
`browse_assert_not_exists`.
````

</details>

<details><summary><code>help("act", "browse_scroll_by")</code> · browse_scroll_by — Scroll by a distance · 586 chars</summary>

Source: [src/actions-reference/90-browser/110-browse_scroll_by.md](src/actions-reference/90-browser/110-browse_scroll_by.md)

````text
### `browse_scroll_by` — Scroll by a distance

```toml
{ action = "browse_scroll_by" }                                  # 300px down, the page
{ action = "browse_scroll_by", direction = "up", amount = 800 }
{ action = "browse_scroll_by", container = ".results", amount = 500 }
```

| Field | Default | Description |
|-------|---------|-------------|
| `direction` | `"down"` | `up`, `down`, `left` or `right` — same values and default as the mobile `swipe`/`scroll` actions |
| `amount` | `300` | Distance in CSS pixels |
| `container` | — | Scroll this element instead of the window |
````

</details>

<details><summary><code>help("act", "browse_scroll_to")</code> · browse_scroll_to — Bring an element into view · 206 chars</summary>

Source: [src/actions-reference/90-browser/120-browse_scroll_to.md](src/actions-reference/90-browser/120-browse_scroll_to.md)

````text
### `browse_scroll_to` — Bring an element into view

```toml
{ action = "browse_scroll_to", selector = "#order-footer" }
```

Useful before a `browse_screenshot`, or for a page that only renders on scroll.
````

</details>

<details><summary><code>help("act", "browse_select")</code> · browse_select — Choose an option in a &lt;select&gt; · 536 chars</summary>

Source: [src/actions-reference/90-browser/130-browse_select.md](src/actions-reference/90-browser/130-browse_select.md)

````text
### `browse_select` — Choose an option in a `<select>`

```toml
{ action = "browse_select", selector = "#status", value = "fulfilled" }
{ action = "browse_select", selector = "#status", text = "Fulfilled" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `value` | — | Match the option's `value` attribute |
| `text` | — | Match the option's visible label instead. Give one or the other, not both |

Fires `input` and `change` the way a user's choice would, so a framework
listening for them sees the update.
````

</details>

<details><summary><code>help("act", "browse_execute_js")</code> · browse_execute_js — Run JavaScript in the page · 831 chars</summary>

Source: [src/actions-reference/90-browser/140-browse_execute_js.llm.md](src/actions-reference/90-browser/140-browse_execute_js.llm.md) (LLM text)

````text
### `browse_execute_js` — Run JavaScript in the page

```toml
{ action = "browse_execute_js", script = "return document.title", save_to = "title" }
{ action = "browse_execute_js", file = "portal-helpers.js", script = "return fulfilOrder('${order_id}')" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `script` | — | Inline JavaScript. `${…}` variables are interpolated |
| `file` | — | `.js` file. Leading `/` = project root, else relative to the flow file. `..` is rejected. `${…}` is **not** interpolated |
| `save_to` | — | Save the returned value. Objects nest (`${result.total}`) |

Give `script`, `file`, or both. The file runs first, in the same evaluation, so the script sees what the file declared. The body runs inside an async function: use `return` to produce the value and `await` freely.
````

</details>

<details><summary><code>help("act", "browse_mcp_list_tools")</code> · browse_mcp_list_tools — List the page's WebMCP tools · 417 chars</summary>

Source: [src/actions-reference/90-browser/150-browse_mcp_list_tools.md](src/actions-reference/90-browser/150-browse_mcp_list_tools.md)

````text
### `browse_mcp_list_tools` — List the page's WebMCP tools

```toml
{ action = "browse_mcp_list_tools", save_to = "tools" }
```

A page that opts into WebMCP (https://developer.chrome.com/docs/ai/webmcp)
describes what it can do — "fulfil an order", "issue a refund" — as named tools
with argument schemas. Saved as an object keyed by tool name, so
`${tools.fulfil_order}` is both a description and a presence check.
````

</details>

<details><summary><code>help("act", "browse_mcp_call")</code> · browse_mcp_call — Call a WebMCP tool · 858 chars</summary>

Source: [src/actions-reference/90-browser/160-browse_mcp_call.md](src/actions-reference/90-browser/160-browse_mcp_call.md)

````text
### `browse_mcp_call` — Call a WebMCP tool

```toml
{ action = "browse_mcp_call", tool = "fulfil_order", arguments = { order_id = "${order_id}" }, save_to = "receipt" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `tool` | — | Tool name, as listed by the page. Required |
| `arguments` | `{}` | Inline table passed to the tool as JSON. Golem `${…}` variables resolve inside it |

A tool the page doesn't register fails with `F404`.

The `{ content: [{ type: "text", … }] }` envelope MCP tools return is unwrapped
— a flow gets the answer, not the scaffolding — and a result that is itself JSON
nests, so `${receipt.order_id}` works.

**Availability.** golem enables WebMCP automatically for flows that contain a
`browse_mcp_*` step. It needs an `https://` or `localhost` page. A browser too
old to support it fails with `H505`.
````

</details>

<details><summary><code>help("act", "browse_cookies")</code> · browse_set_cookie / browse_get_cookie — Cookies · 732 chars</summary>

Source: [src/actions-reference/90-browser/170-browse_cookies.md](src/actions-reference/90-browser/170-browse_cookies.md)

````text
### `browse_set_cookie` / `browse_get_cookie` — Cookies

```toml
{ action = "browse_set_cookie", name = "session", value = "${portal_session}" }
{ action = "browse_set_cookie", name = "region", value = "eu", domain = "portal.example.com", path = "/" }
{ action = "browse_get_cookie", name = "session", save_to = "portal_session" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `name` | — | Cookie name. Required |
| `value` | — | Required for `browse_set_cookie` |
| `domain` | current page | Restrict the cookie to a domain |
| `path` | current page | Restrict the cookie to a path |

`browse_get_cookie` reads `HttpOnly` cookies too. A `browse_get_cookie` for a
name that isn't set fails with `F404`.
````

</details>

<details><summary><code>help("act", "browse_local_storage")</code> · browse_set_local_storage / browse_get_local_storage — Local storage · 647 chars</summary>

Source: [src/actions-reference/90-browser/180-browse_local_storage.md](src/actions-reference/90-browser/180-browse_local_storage.md)

````text
### `browse_set_local_storage` / `browse_get_local_storage` — Local storage

```toml
{ action = "browse_set_local_storage", key = "feature_flags", value = "{\"beta\": true}" }
{ action = "browse_get_local_storage", key = "session", save_to = "session" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `key` | — | Required |
| `value` | — | Required for the setter |

**Reading parses JSON objects.** Web apps keep structured state in storage as
JSON text, so an object nests and `${session.user.id}` works. Anything else — an
array, a number, plain text — stays the text it was. A key that isn't set fails
with `F404`.
````

</details>

<details><summary><code>help("act", "browse_session_storage")</code> · browse_set_session_storage / browse_get_session_storage — Session storage · 302 chars</summary>

Source: [src/actions-reference/90-browser/190-browse_session_storage.md](src/actions-reference/90-browser/190-browse_session_storage.md)

````text
### `browse_set_session_storage` / `browse_get_session_storage` — Session storage

```toml
{ action = "browse_set_session_storage", key = "step", value = "2" }
{ action = "browse_get_session_storage", key = "step", save_to = "step" }
```

Identical to the local-storage pair, against `sessionStorage`.
````

</details>

<details><summary><code>help("act", "browse_close")</code> · browse_close — Close a tab early · 206 chars</summary>

Source: [src/actions-reference/90-browser/200-browse_close.md](src/actions-reference/90-browser/200-browse_close.md)

````text
### `browse_close` — Close a tab early

```toml
{ action = "browse_close" }
{ action = "browse_close", session = "admin" }
```

Optional. Every tab is closed when the flow ends; this hands one back sooner.
````

</details>

</details>

<details><summary><code>help("act", "flow-control")</code> · Flow Control · 104 chars · + 1 item</summary>

Source: [src/actions-reference/100-flow-control/00-intro.md](src/actions-reference/100-flow-control/00-intro.md)

```text
## Flow Control

End the flow on purpose.

Items (help("act", item)):
- fail: Fail the flow immediately
```

<details><summary><code>help("act", "fail")</code> · fail — Fail the flow immediately · 455 chars</summary>

Source: [src/actions-reference/100-flow-control/10-fail.md](src/actions-reference/100-flow-control/10-fail.md)

````text
### `fail` — Fail the flow immediately

```toml
{ action = "fail", message = "Unexpected state reached" }
{ action = "fail", message = "Bad total: ${order.total}" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `message` | `"Flow failed (no message provided)"` | Failure reason shown in reports; supports inline `${…}` vars |

The only field `fail` uses is `message`. Useful in conditional branches to mark
unreachable paths.
````

</details>

</details>

</details>


### selectors

<details><summary><code>help("selectors")</code> · Selectors · 1202 chars · + 10 items</summary>

Source: [src/selectors/00-intro.md](src/selectors/00-intro.md)

```text
# Selectors

A selector describes *which* on-screen element a step targets. golem resolves it
against the **visible tree** — the elements a human can actually see (clipped to
ancestor containers).
The same selector grammar is used everywhere an element is named: `tap`,
`assert_visible`, `read`, `scroll`'s `to`/`within`, swipe points, etc.

Items (help("selectors", item)):
- two-syntaxes: Flat on_* keys, or grouped on = { … } for traits, contains, inside
- core: on_text, on_accessibility_label, on_index; what counts as text; when to prefer text
- state-filters: Filter by enabled, checked or clickable
- traits: Shape and text predicates: button, wide, tall, has_text and more
- relational: below, above, right_of or left_of a visible anchor
- containment: contains or inside: select by nested bounds, not DOM; min_matches for a list
- chaining: Nested anchors, AND of keys, how ties sort and index picks
- occlusion: tap hit-tests and routes around overlays such as sticky headers
- within: Scope a scroll to a region: below a heading, or a list container
- canvas: Compose and Flutter UIs: what works and what differs by platform — 4 items: help("selectors", "canvas") lists them; read it first
```

<details><summary><code>help("selectors", "two-syntaxes")</code> · Two syntaxes · 359 chars</summary>

Source: [src/selectors/10-two-syntaxes.md](src/selectors/10-two-syntaxes.md)

````text
## Two syntaxes

**Flat** (`on_*` fields) for simple cases:

```toml
{ action = "tap", on_text = "Submit", on_below = "Counter" }
```

**Grouped** (`on = { … }`, also `to = { … }` / `within = { … }`), required for
`traits`, `contains`, `inside`, and nested anchors:

```toml
{ action = "tap", on = { text = "Submit", below = "Counter", enabled = true } }
```
````

</details>

<details><summary><code>help("selectors", "core")</code> · Core selectors · 1677 chars</summary>

Source: [src/selectors/20-core.md](src/selectors/20-core.md)

```text
## Core selectors

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_text` | `text` | Visible text. Glob (`*`, `?`), case-insensitive, anchored (full-string — use globs for partial: `"Item *"`, `"*@*"`). |
| `on_accessibility_label` | `accessibility_label` | The element's accessibility label (aria-label, Android `contentDescription`) or its identifier (iOS `accessibilityIdentifier`, Android `resource-id`). `golem tree` shows these as `label=` and `id=`. **See the guidance below — prefer `text`.** |
| `on_index` | `index` | The Nth match (0-based) after all other filters. |

**What counts as text.** `text` is the text the platform reports as visible.
When an element has no text of its own, its accessibility label is its text,
on both platforms. Thus an icon-only button labelled `Close` matches
`on_text = "Close"`. An image (Android `ImageView`, iOS `image`) is the
exception: its label describes a picture, so it is never text. In a webview,
`text` is the DOM text only.

### Prefer visible `text`; use `accessibility_label` sparingly

Default to `on_text` (or a positional/`contains` selector). Use
`on_accessibility_label` only:

1. **When the test is about the accessibility label itself** — e.g.
   validating screen-reader semantics / a11y compliance. Here the label *is* the
   thing under test.
2. **To navigate to the screen under test, when the tapped element is not what
   the step checks** — e.g. opening a menu by its stable `menu-toggle` id.

Outside those cases, an `accessibility_label` selector tests something the user
never perceives, and silently passes even if the visible text is wrong. When in
doubt, use `text`.
```

</details>

<details><summary><code>help("selectors", "state-filters")</code> · State filters · 296 chars</summary>

Source: [src/selectors/30-state-filters.md](src/selectors/30-state-filters.md)

```text
## State filters

| Selector | Grouped key | Matches |
|----------|-------------|---------|
| `on_enabled` | `enabled` | Enabled state (`true`/`false`). |
| `on_checked` | `checked` | Checked/selected state (`true`/`false`). |
| `on_clickable` | `clickable` | Clickable state (`true`/`false`). |
```

</details>

<details><summary><code>help("selectors", "traits")</code> · Traits · 579 chars</summary>

Source: [src/selectors/40-traits.md](src/selectors/40-traits.md)

````text
## Traits

Computed predicates on an element's geometry and content. All listed traits in a
selector must hold (AND).

```toml
{ action = "assert_visible", on = { text = "Submit", traits = ["button", "wide"] } }
```

| Trait | True when |
|-------|-----------|
| `button` | Element type is a button or link. |
| `has_text` / `text` | Has non-empty text. |
| `no_text` | Has no text. |
| `short_text` | Text length 1–10. |
| `long_text` | Text length > 50. |
| `square` | Width/height ratio between 0.8 and 1.2. |
| `wide` | Width > 2 × height. |
| `tall` | Height > 2 × width. |
````

</details>

<details><summary><code>help("selectors", "relational")</code> · Relational (positional) selectors · 1163 chars</summary>

Source: [src/selectors/50-relational.md](src/selectors/50-relational.md)

````text
## Relational (positional) selectors

Locate an element by its position relative to a visible **anchor**:

| Selector | Grouped key | Keeps elements… |
|----------|-------------|-----------------|
| `on_below` | `below` | below the anchor's bottom |
| `on_above` | `above` | above the anchor's top |
| `on_right_of` | `right_of` | right of the anchor's right edge |
| `on_left_of` | `left_of` | left of the anchor's left edge |

```toml
{ action = "assert_visible", on_text = "2", on_below = "Counter" }
```

Two rules make these behave the way a human reads layout:

- **Cross-axis overlap is required.** `below`/`above` also require the candidate
  to **horizontally overlap** the anchor; `left_of`/`right_of` require **vertical
  overlap**. So "below the heading" means below *and in the heading's column* —
  an element in another column (e.g. a two-column tablet layout) is not matched.
  Any positive overlap counts.
- **Nearest-first.** Among matches, the one closest to the anchor (by gap along
  the relation's axis) comes first.

The anchor must be **on-screen**. If it exists but is scrolled off, the
relational match is treated as unresolved (empty).
````

</details>

<details><summary><code>help("selectors", "containment")</code> · Geometric containment: contains / inside · 1954 chars</summary>

Source: [src/selectors/60-containment.md](src/selectors/60-containment.md)

````text
## Geometric containment: `contains` / `inside`

Select by spatial nesting — coordinate-based, *not* DOM structure. There is no
parent/child selector.

| Grouped key | Keeps elements whose bounds… |
|-------------|------------------------------|
| `contains` | **fully enclose** the anchor (the box *around* X) |
| `inside` | are **fully enclosed by** the anchor (things *within* a region) |

```toml
# the container that holds "Item 0" (smallest such box)
{ action = "assert_visible", on = { contains = { text = "Item 0" } } }
# an item inside a labelled region
{ action = "assert_visible", on = { text = "Item 0", inside = { accessibility_label = "section-scroll-list" } } }
```

`contains` excludes the anchor itself and a wrapper with exactly the anchor's
bounds, and resolves **smallest-enclosing first**.

### `min_matches` — the container of *repeated* items

The smallest box enclosing a *single* item is often a per-item wrapper (a
`<li>`, a list cell), not the list one level up. To target the
**container of several repeated items**, give the `contains` group form a
`min_matches`:

```toml
# the smallest element that encloses ≥2 "Row *" matches — the list, not one row's wrapper
{ action = "assert_visible", on = { contains = { text = "Row *", min_matches = 2 } } }
```

Semantics: *the smallest visible element whose bounds enclose ≥ `min_matches`
elements matching the anchor.* It counts only **visible** matches
(off-screen items are filtered), so the result is the list's on-screen
box. `min_matches` defaults to `1` and must be `1`–`100` (a larger value is
rejected at parse time). `min_matches` is valid **only** on `contains`. To scope
a scroll to a list this way, see `within` (help("selectors", "within")).

> If a list is so short that only one item is visible, `min_matches = 2` can't
> resolve it — but neither could a human see it's a scrollable list. Make the
> list taller, or fall back to `within = { below = "<heading>" }`.
````

</details>

<details><summary><code>help("selectors", "chaining")</code> · Nesting and chaining · 648 chars</summary>

Source: [src/selectors/70-chaining.llm.md](src/selectors/70-chaining.llm.md) (LLM text)

```text
## Nesting and chaining

- An anchor (`below`, `above`, `left_of`, `right_of`, `contains`, `inside`) is bare text or a full selector group, and a group may have its own anchors: `{ text = "Left", below = { text = "Nested Layout", traits = ["has_text"] } }`. The anchor is the group's first match, and it must be on screen.
- Every key in a group is combined with AND.
- Survivors sort: containment tightest-first, then nearest to the directional anchor, then tree order. `index` (0-based) picks from that list.
- golem does not guess between equal candidates: a tie resolves by tree order. To pick a specific one, add `index` or another predicate.
```

</details>

<details><summary><code>help("selectors", "occlusion")</code> · Occlusion-aware tapping · 1045 chars</summary>

Source: [src/selectors/80-occlusion.md](src/selectors/80-occlusion.md)

```text
## Occlusion-aware tapping

The visible tree tells golem what's *clipped or off-screen*, but not what's *covered*
by something painted on top (a sticky header, a `z-index` overlay). So golem
**hit-tests** the target before tapping and **routes around** an occluder: a plain
`tap` lands on the first clear sample point, so a button
whose centre sits under a sticky header still gets hit on a clear edge. The routed
coordinate shows in the `--verbose` `element_resolved` substep (`tap=(x,y)`).

Two guarantees:

- **It never blocks.** Occlusion is a heuristic — golem always attempts the tap; if
  no sampled point is clear it falls back to the centre. Treat a reported occlusion
  as *"may be covered"*, not a hard failure.
- **Offsets stay centre-relative.** `x`/`y` offsets are always measured from the
  element's geometric centre, never the routed point — so they stay predictable
  regardless of what's covering the element.

This detects layout/paint occlusion only — an element under the OS status bar is a
separate, system-level concern.
```

</details>

<details><summary><code>help("selectors", "within")</code> · within (scoping a scroll) · 665 chars</summary>

Source: [src/selectors/90-within.md](src/selectors/90-within.md)

````text
## `within` (scoping a scroll)

`scroll`'s `within = { … }` names the region to scroll inside. It uses the same
selector grammar. Two robust idioms for an inner list:

```toml
# 1. heading-relative — scope to what's below a heading
{ action = "scroll", to = { text = "Item 45" }, within = { below = "Scroll List" } }

# 2. repeated-item container — scope to the box holding ≥2 matching items
#    (use when items are wrapped, e.g. <li>, and `below` isn't convenient)
{ action = "scroll", to = { text = "Row 45" }, within = { contains = { text = "Row *", min_matches = 2 } } }
```

See `min_matches` and
Actions Reference → scroll (help("act")) for the full action.
````

</details>

<details><summary><code>help("selectors", "canvas")</code> · Canvas-rendered UI (Compose, Compose Multiplatform, Flutter) · 2253 chars · + 4 items</summary>

Source: [src/selectors/100-canvas/00-intro.llm.md](src/selectors/100-canvas/00-intro.llm.md) (LLM text)

```text
## Canvas-rendered UI (Compose, Compose Multiplatform, Flutter)

Compose, Compose Multiplatform and Flutter copy their semantics tree into the platform accessibility tree, so golem sees one node per widget. Text, relational and containment selectors work on these nodes.

- **Select by visible text.** It is the one path that works the same on Android and iOS.
- **Identifiers.** A Compose `Modifier.testTag` or a Flutter `Semantics(identifier:)` is matched by `accessibility_label`. On Android, a Compose `testTag` reaches golem only with `testTagsAsResourceId = true` on an ancestor. Flutter needs 3.19 or later; Compose Multiplatform on iOS needs 1.8.0 or later.
- **`contentDescription` on iOS.** It merges into the element's text (`"Increment, +"`). On a `Text` it replaces the visible text, so do not put one on a `Text` whose value a step checks.
- **Element types.** On Compose, element types are not reliable, so neither is the `button` trait: `{ text = "+", traits = ["button"] }` matches nothing. Use `{ action = "tap", on_text = "+" }`.
- **Scrolled-out nodes on iOS.** A non-lazy Compose layout (a `Column` with `verticalScroll`) and a Flutter `ListView` keep off-screen nodes in the iOS tree. The viewport filter drops them, so `assert_not_visible` treats them as absent. A non-lazy Compose node hidden by a scroll container smaller than the screen keeps its full `visible_bounds`, so golem counts it as on screen.
- **Merged semantics.** On Compose, child text nodes stay in the tree: target a child's text (`on_text = "Click B"`); anchor `inside`/`contains` on a child's text, not the container's. On Flutter, `MergeSemantics` removes the children and the container's text is the children's text joined by a line break: select it with a glob (`on_text = "Merged A*"`).
- **Flutter custom widgets** (a `GestureDetector` on a `Container`, a `CustomPaint`) are invisible to golem until wrapped in `Semantics`.

Items (help("selectors", item)):
- annotations: Which Compose or Flutter annotation becomes text or label
- element-types: element_type and the button trait are unreliable on canvas UIs
- bounds: Off-screen nodes: left out on Android, kept on iOS
- merged-semantics: How merged nodes report their text, by framework and platform
```

<details><summary><code>help("selectors", "annotations")</code> · Which annotation reaches golem · 1229 chars</summary>

Source: [src/selectors/100-canvas/20-annotations.md](src/selectors/100-canvas/20-annotations.md)

```text
### Which annotation reaches golem

| Annotation | Android | iOS |
|------------|---------|-----|
| Compose `contentDescription` | `accessibility_label`, on its own node, which has no other text, so the description is also that node's `text`. The visible text stays a separate text node. | Merged into the element's text. A button with text `+` and description `Increment` reads `"Increment, +"`. On a `Text`, the description **replaces** the visible text. |
| Compose `Modifier.testTag` | Identifier (`resource-id`), only with `testTagsAsResourceId = true` on an ancestor. | Identifier (`accessibilityIdentifier`), with no opt-in. |
| Flutter `Semantics(identifier:)` | Identifier (`resource-id`). | Identifier (`accessibilityIdentifier`). |
| Flutter widget text (`Text`, button text) | The label; golem reads it as `text` (see What counts as text (help("selectors", "core"))). | `text` |
| Visible text | `text` | `text` |

The `accessibility_label` selector matches the label or the identifier.

Visible text is the one path that works the same on both platforms. Use it (see
Prefer visible `text`). Do not put a
`contentDescription` on a `Text` whose value a step checks, because on iOS the
step then reads the description.
```

</details>

<details><summary><code>help("selectors", "element-types")</code> · Element types · 1086 chars</summary>

Source: [src/selectors/100-canvas/30-element-types.md](src/selectors/100-canvas/30-element-types.md)

```text
### Element types

`element_type`, and the `button` trait that reads it, are not reliable on a
canvas UI:

- **Android (Compose):** a `Text` is a `TextView`, and every other node is a
  `View`. A Material `Button` adds a separate `Button` node. That node has no text
  or label and is not clickable. The clickable node is the parent `View`, and the
  button's text is a child `TextView`.
- **iOS (Compose Multiplatform):** nodes get real types (`button`, `text`,
  `other`). But the `button` node's text is the merged string, for example
  `"Increment, +"`, and the visible `+` is a child `text` node.
- **Flutter:** a button is a `Button` (Android) or `button` (iOS) node that
  holds its own text. A `Text` is a plain `View` on Android and `text` on iOS.
  An `Icon` with a `semanticLabel` looks the same as a `Text` on both
  platforms. Only `Semantics(image: true)` and `Image` give an image node
  (`ImageView` / `image`).

As a result, on Compose `{ text = "+", traits = ["button"] }` matches nothing on
either platform. Select by text alone: `{ action = "tap", on_text = "+" }`.
```

</details>

<details><summary><code>help("selectors", "bounds")</code> · Bounds and visibility · 1010 chars</summary>

Source: [src/selectors/100-canvas/40-bounds.md](src/selectors/100-canvas/40-bounds.md)

```text
### Bounds and visibility

Every Compose and Flutter node carries `bounds` and `visible_bounds` on both
platforms. golem's viewport filter drops off-screen nodes, and `scroll` brings
them into view. Compose has two differences between the platforms:

- **Android** leaves off-screen Compose nodes out of the tree.
- **iOS** keeps every node of a non-lazy layout (for example a `Column` with
  `verticalScroll`) in the tree, with `visible_bounds` equal to `bounds`. That
  is, Compose Multiplatform does not clip `visible_bounds` to the scroll
  container.

A lazy Flutter `ListView` leaves its off-screen items out of the Android tree.
On iOS it keeps them, with a zero-size frame at the origin.

Both kinds of node fall outside the viewport, so `assert_not_visible` treats
them as absent on both platforms, as a user would. One caveat remains on iOS:
a non-lazy Compose node that its scroll container hides but that is still
inside the screen keeps its full `visible_bounds`, so golem counts it as on
screen.
```

</details>

<details><summary><code>help("selectors", "merged-semantics")</code> · Merged semantics · 1250 chars</summary>

Source: [src/selectors/100-canvas/50-merged-semantics.md](src/selectors/100-canvas/50-merged-semantics.md)

```text
### Merged semantics

`Modifier.semantics(mergeDescendants = true)`, `Modifier.clickable` and Material
components merge their children into one accessibility node. Flutter's
`MergeSemantics` does the same, but the two frameworks report the result
differently:

| | Compose, Android | Compose, iOS | Flutter, both platforms |
|---|---|---|---|
| Merged container | No text | Text is the children's text, joined: `"Click A, Click B"` | Text is the children's text, joined by a line break: `"Merged A\nMerged B"` |
| Child text nodes | Present | Present | **Absent** |

On Compose, child text nodes stay in the tree on both platforms. Thus these
selectors work on both:

- `on_text` on a child: `{ action = "tap", on_text = "Click B" }` taps inside the
  clickable container.
- `below` / `above` / `left_of` / `right_of` anchored on a child's text.

`inside` and `contains` anchored on the **container's** text work only on iOS,
because the Android container has no text. Anchor on a child's text, or on a
heading, instead.

On Flutter, a merged child has no node of its own. Select the container by its
joined text, with a glob: `{ action = "tap", on_text = "Merged A*" }`. `inside`
and `contains` are geometric, so a coarse tree does not change them.
```

</details>

</details>

</details>


### flow

<details><summary><code>help("flow")</code> · Test Structure · 1317 chars · + 14 items, 14 sub-items</summary>

Source: [src/test-structure/00-intro.md](src/test-structure/00-intro.md)

```text
# Test Structure

Tests are written in TOML. A `.test.toml` file defines a **flow** — the top-level unit of execution.

Items (help("flow", item)):
- flow: [flow] fields, [[flow.apps]], vars and options — launch-permissions, options, accessibility-audit, performance-monitoring
- block: Blocks group steps into logical sections — platform, branching, next
- step: One action with selectors, timeout and error handling — selectors, options, timeout-multipliers
- subflow: run_flow: a block runs another flow file on the same device
- reuse: When to use a subflow, a mixin or a fixture
- lifecycle: Setup is implicit: install, then app_lifecycle reset or launch
- teardown: [[teardown]] steps run after the flow, even when it fails
- data-driven: [[data]] rows; for_each = "data" runs a block per row
- variables: Set by --var, [flow.vars], rows, save_to or fixtures; read as ${name}; built-ins such as _device, _os
- secrets: secret in a fixture or on a save_to step: golem masks the value in all output
- fake-data: ${fake:…} in short; help("fake") has the rest
- multi-app: Several apps in one flow; switch with launch app = "…"
- devices: [[flow.apps.devices]]: os, type, hardware and name constraints — coverage, hardware, pinning, auto-boot
- project-config: golem.toml: project defaults that every flow inherits
```

<details><summary><code>help("flow", "flow")</code> · Flow · 1415 chars · + 4 items</summary>

Source: [src/test-structure/10-flow/00-intro.md](src/test-structure/10-flow/00-intro.md)

````text
## Flow

A flow is a complete test scenario: metadata, app configuration, device targets, execution blocks, and optional teardown.

```toml
[flow]
name = "Login test"
tags = ["auth", "smoke"]
# start = "block_name"  # Optional: skip to this block (assumes app in correct state)

[[flow.apps]]
name = "app"
bundle = "com.example.myapp"

[[flow.apps.devices]]
os = "ios:latest"
type = "phone"

[[flow.apps.devices]]
os = "android:latest"
type = "phone"

[[block]]
name = "login"
steps = [
  { action = "type", on_text = "Email", input = "user@example.com" },
  { action = "type", on_text = "Password", input = "secret" },
  { action = "tap", on_text = "Sign In" },
  { action = "assert_visible", on_text = "Dashboard", timeout = 10000 },
]
```

`[[flow.apps.devices]]` says which devices the flow runs on — see Devices (help("flow", "devices")).

The `name` you give an app here is how any action with an `app` field (`launch`, `stop`, `clear_data`, `push_notification`, …) refers to it: `app = "app"`. Bundle ids also work. See Multi-App Flows (help("flow", "multi-app")).

Items (help("flow", item)):
- launch-permissions: Grant app permissions before the app launches
- flow/options: [flow.options]: timeouts, limits, recording, coverage, perf
- accessibility-audit: Per-block audit of the visible tree; levels and thresholds
- performance-monitoring: Per-block memory, CPU, threads and fds; warn and error limits
````

<details><summary><code>help("flow", "launch-permissions")</code> · Launch-time permissions · 827 chars</summary>

Source: [src/test-structure/10-flow/05-launch-permissions.md](src/test-structure/10-flow/05-launch-permissions.md)

````text
### Launch-time permissions

An app entry may declare `permissions`. golem grants them **before the app is launched**, so they are in place from process start. A grant that fails is a warning, not a failure: the app then shows its permission prompt when it needs the permission.

```toml
[[flow.apps]]
name = "app"
permissions = { photos = "allow", camera = "deny" }
```

Keys use the cross-platform permission names (`camera`, `microphone`, `location`, `photos`, …). The value is a **mode**: `"allow"` or `"deny"` for any permission, plus `location = "always"` (background; `allow` is foreground) and `photos = "limited"`. To change a permission later in a flow, relaunch the app with the `launch` action's own `permissions =` map. App permissions (help("act", "app-permissions")) owns the full permission and mode reference.
````

</details>

<details><summary><code>help("flow", "flow/options")</code> · Flow Options · 1984 chars</summary>

Source: [src/test-structure/10-flow/10-options.md](src/test-structure/10-flow/10-options.md)

````text
### Flow Options

```toml
[flow.options]
step_timeout = 5000                 # Base timeout (ms), default: 5000. See Timeout Multipliers.
max_steps = 10000                   # Fail the flow after this many steps (default: 10000)
max_runtime = "30m"                 # Fail the flow after this long (default: "1h"). "5m", "2h", "500ms"
max_device_wait = "30m"             # Fail if no device frees up within this time (default: wait forever). --max-device-wait overrides.
app_lifecycle = "reset"             # "reset" (default), "launch", "manual" — see Lifecycle
screenshot_on_failure = true        # Auto-capture screenshot on step failure (default: true)
record = true                       # Default every block to record (block can opt out with `record = false`)
coverage = "smart"                  # "smart" (default), "min", "full", "one" — see Coverage Strategies
create_if_missing = false           # Create a simulator/emulator when none matches (default: false) — see Hardware Axis
perf = true                         # Performance monitoring (default: true)
a11y = "relaxed"                    # Accessibility audit: "off", "critical", "relaxed" (default), "strict". --a11y overrides.
```

Thresholds are unset by default; the values below are examples. A `*_warn*` threshold adds a warning; a `*_error*` threshold fails the flow.

```toml
[flow.options]
perf_memory_warn_mb = 200.0         # App memory, MB
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0        # App CPU, %
perf_cpu_error_percent = 95.0
perf_threads_warn = 100             # Thread count
perf_threads_error = 200
perf_fd_warn = 200                  # Open file descriptors
perf_fd_error = 500
a11y_max_errors = 0                 # Fail the flow if cumulative a11y errors exceed this
a11y_max_warnings = 20              # Fail the flow if cumulative a11y warnings exceed this
a11y_min_confidence = 0.8           # Drop a11y findings below this confidence (0–1). Deterministic checks are 1.0.
```
````

</details>

<details><summary><code>help("flow", "accessibility-audit")</code> · Accessibility Audit · 602 chars</summary>

Source: [src/test-structure/10-flow/20-accessibility-audit.md](src/test-structure/10-flow/20-accessibility-audit.md)

```text
### Accessibility Audit

After each block, Golem audits the **visible** UI tree for accessibility issues
(zero config, on by default at `relaxed`). Findings appear inline in the live run
and in every report format. They are warnings unless a flow sets the `a11y_*`
thresholds in Flow Options (help("flow", "flow/options")). Levels: `off`, `critical` (tree
checks only), `relaxed` (default), `strict` (adds the screenshot contrast check +
an annotated screenshot).

Full guide — the checks, per-level thresholds, the confidence model, and how to
read the annotated screenshot — in **accessibility.md**.
```

</details>

<details><summary><code>help("flow", "performance-monitoring")</code> · Performance Monitoring · 609 chars</summary>

Source: [src/test-structure/10-flow/30-performance-monitoring.md](src/test-structure/10-flow/30-performance-monitoring.md)

```text
### Performance Monitoring

Golem captures app performance metrics after each block (unless `--no-perf` or `perf = false`).

| Metric | Unit |
|--------|------|
| Memory | MB |
| CPU | % |
| Threads | count |
| File descriptors | count |
| Disk | MB |
| Network RX/TX | KB |

The `perf_*` thresholds in Flow Options (help("flow", "flow/options")) act on memory, CPU, threads and file descriptors: crossing a warn threshold adds a warning, crossing an error threshold fails the flow.

Performance data appears in all output formats: human (table), JSON (objects), JUnit (properties), toon (abbreviated codes).
```

</details>

</details>

<details><summary><code>help("flow", "block")</code> · Block · 1063 chars · + 3 items</summary>

Source: [src/test-structure/20-block/00-intro.md](src/test-structure/20-block/00-intro.md)

````text
## Block

Blocks group steps into logical sections. They execute in document order by default.

```toml
[[block]]
name = "setup"
steps = [
  { action = "assert_visible", on_text = "Welcome", timeout = 30000 },
]

[[block]]
name = "main_test"
steps = [
  { action = "tap", on_text = "+" },
  { action = "tap", on_text = "+" },
  { action = "assert_visible", on_text = "2", on_below = "Counter" },
]
```

`record = true` / `false` on a block overrides the flow's `record` default for that block (`--no-record` still wins).

`max_iterations = N` on a block fails the flow with `EF509` when the flow would run the block an (N+1)th time. It guards a block that loops through `next`, a branch `goto` or `[[data]]` rows: each row counts as one run. To leave a loop on purpose instead, branch on `_loop` (see Branching (help("flow", "branching"))).

Items (help("flow", item)):
- platform: where: run a block only on devices that match
- branching: if_visible, if_not_visible or if_var, then goto a block
- next: Go to a named block after this one; ignored with branches
````

<details><summary><code>help("flow", "platform")</code> · Platform-Specific Blocks · 663 chars</summary>

Source: [src/test-structure/20-block/10-platform.md](src/test-structure/20-block/10-platform.md)

````text
### Platform-Specific Blocks

`where` runs a block only on devices that match it; on other devices the block is skipped:

```toml
[[block]]
name = "android_back"
where = { os = "android" }
steps = [
  { action = "press", button = "back" },
]
```

| Key | Matches |
|---|---|
| `os` | `"ios"` / `"android"` — the platform; any other value is an OS-version prefix (`"17"` matches 17.x) |
| `type` | `"phone"` / `"tablet"` |
| `physical` | `true` on a physical device, `false` on a simulator/emulator |

A device must match every key given. A skipped block's `next` and `[[block.branch]]` are not evaluated: the flow continues with the next block in document order.
````

</details>

<details><summary><code>help("flow", "branching")</code> · Branching · 1292 chars</summary>

Source: [src/test-structure/20-block/20-branching.md](src/test-structure/20-block/20-branching.md)

````text
### Branching

Control flow between blocks with conditions:

```toml
[[block]]
name = "check_state"
steps = [
  { action = "assert_visible", on_text = "Welcome", if_fail = "ignore" },
]

[[block.branch]]
if_visible = "Dashboard"
goto = "already_logged_in"

[[block.branch]]
goto = "login_required"            # Unconditional fallback
```

Branches are checked after the block's steps finish (for a `for_each` block, after the last row; for a `run_flow` block, after the child flow returns). They are checked in order, and the first match wins. If none matches, the flow continues with the next block in document order.

| Condition | Matches when |
|---|---|
| `if_visible = "…"` | an element with this text (glob) is on screen |
| `if_not_visible = "…"` | no element with this text is on screen |
| `if_var = "x"`, `equals = "…"` | the variable equals the string exactly |
| `if_var = "x"`, `matches = "…"` | the variable matches the glob pattern |
| `if_var = "x"`, `gte = N` | the variable, read as an integer, is ≥ `N` (an integer). A non-integer value never matches. |
| (no condition) | always |

`if_var` without `equals`, `matches` or `gte` never matches. A block with any `[[block.branch]]` ignores its `next`, even when no branch matches — put an unconditional `goto` last instead.
````

</details>

<details><summary><code>help("flow", "next")</code> · Block next · 334 chars</summary>

Source: [src/test-structure/20-block/30-next.md](src/test-structure/20-block/30-next.md)

````text
### Block `next`

Jump to a named block after completion (instead of falling through). A block with `[[block.branch]]` entries ignores `next`.

```toml
[[block]]
name = "step_a"
next = "step_c"
steps = [...]

[[block]]
name = "step_b"
steps = [...]    # Skipped

[[block]]
name = "step_c"
steps = [...]    # Executed after step_a
```
````

</details>

</details>

<details><summary><code>help("flow", "step")</code> · Step · 614 chars · + 3 items</summary>

Source: [src/test-structure/30-step/00-intro.md](src/test-structure/30-step/00-intro.md)

````text
## Step

A step is a single action with optional selectors, timeouts, and error handling.

```toml
{ action = "tap", on_text = "Submit" }
{ action = "assert_visible", on_text = "1", on_below = "Counter", timeout = 5000 }
{ action = "type", on_text = "Email", input = "hello@example.com" }
{ action = "read", on_right_of = "Status:", save_to = "status_value" }
```

Items (help("flow", item)):
- selectors: Selector keys on a step; prefer on_text
- step/options: timeout, auto_scroll, if_fail, retry, save_to and other step fields
- timeout-multipliers: Each action's default timeout, as a multiple of step_timeout
````

<details><summary><code>help("flow", "selectors")</code> · Selectors · 675 chars</summary>

Source: [src/test-structure/30-step/10-selectors.md](src/test-structure/30-step/10-selectors.md)

````text
### Selectors

Find the element a step acts on. Prefer `on_text`; use `on_accessibility_label` only to test the a11y label.

| Selector | Description |
|----------|-------------|
| `on_text` | Match by visible text (glob, case-insensitive). **Preferred.** |
| `on_accessibility_label` | Match the accessibility label or the identifier (glob). |
| `on_below` / `on_above` / `on_right_of` / `on_left_of` | Position relative to an anchor element |

```toml
{ action = "tap", on_text = "Submit", on_below = "Counter" }
```

**See Selectors (help("selectors"))** for every selector, state filter and trait, the grouped `on = { … }` form, containment, and how a match is resolved.
````

</details>

<details><summary><code>help("flow", "step/options")</code> · Step Options · 1117 chars</summary>

Source: [src/test-structure/30-step/20-options.md](src/test-structure/30-step/20-options.md)

```text
### Step Options

| Field | Default | Description |
|-------|---------|-------------|
| `timeout` | per-action | Max wait in ms. Overrides computed default. |
| `auto_scroll` | `false` | Scroll page to find element |
| `scroll_timeout` | — | With `scroll` or `auto_scroll`: stop scrolling after this many ms. The step `timeout` still applies. |
| `within` | — | With `scroll` or `auto_scroll`: scroll only inside the element this selector matches — see `within` (help("selectors", "within")) |
| `keep_keyboard` | `false` | Leave the soft keyboard up: golem does not dismiss it before a tap or when it hides the target |
| `if_fail` | `"error"` | `"error"` (fail flow), `"warn"` (log + continue), `"ignore"` (silent continue) |
| `retry` | `0` | Retry count on failure |
| `retry_delay` | `1000` | Delay between retries (ms) |
| `save_to` | — | Save result to a variable |
| `secret` | — | With `save_to`: `true` masks the whole saved value in all output, and a list of paths, such as `["token"]`, masks only those — see Secrets (help("flow", "secrets")) |
| `app` | — | Target a specific app (for multi-app flows) |
```

</details>

<details><summary><code>help("flow", "timeout-multipliers")</code> · Timeout Multipliers · 1251 chars</summary>

Source: [src/test-structure/30-step/30-timeout-multipliers.md](src/test-structure/30-step/30-timeout-multipliers.md)

```text
### Timeout Multipliers

Each action has a built-in multiplier applied to the base timeout (`step_timeout`, default 5000ms). Per-step `timeout` always overrides.

| Multiplier | Timeout (at 5s base) | Actions |
|------------|---------------------|---------|
| 1x | 5s | `screenshot`, `add_media`, `fail`, `load_fixture`, `push_notification`, `clear_data`, `press`, `set_dark_mode`, `set_location`, `hide_keyboard` |
| 2x | 10s | `tap`, `double_tap`, `backspace`, `clear_text`, `long_press`, `swipe`, `pinch`, `gesture`, `rotate`, `type`, `assert_visible`, `assert_not_visible`, `read`, `assert_alert`, `accept_alert`, `dismiss_alert`, and any action not listed here |
| 4x | 20s | `bash`, `run` |
| 5x | 25s | `launch`, `stop` |
| 6x | 30s | `get_http`, `post_http`, `put_http`, `patch_http`, `delete_http`, `open_link`, `create_inbox` |
| 8x | 40s | `scroll` (12x with `within`) |
| 48x | 240s | `await_email` |

The `auto_scroll = true` option sets 8x on any action (12x with `within`).

Actions that take time by themselves (`long_press`, `swipe` through 3+ points, `gesture`, `rotate`, `type`, `backspace`, `clear_text`) get at least that time plus 2s: `max(multiplied, duration + 2s)`. For `type` and `backspace` the time is 500ms per character.
```

</details>

</details>

<details><summary><code>help("flow", "subflow")</code> · Subflow · 1083 chars</summary>

Source: [src/test-structure/40-subflow.llm.md](src/test-structure/40-subflow.llm.md) (LLM text)

````text
## Subflow

A block with `run_flow` runs another flow file instead of its own steps. The child sees the parent's variables and runs on the same device.

```toml
# parent.test.toml
[[block]]
name = "increment"
run_flow = "subflows/increment_counter.test.toml"   # path relative to this flow

[block.vars]                         # set in the child
step = "1"

[block.save_to]                      # child variable = parent variable to write
counter_value = "result_after_increment"
```

```toml
# subflows/increment_counter.test.toml
[flow]
name = "Increment counter"
explicit_only = true

[flow.options]
app_lifecycle = "manual"

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "read", on_below = "Counter", save_to = "counter_value" },
]
```

- If the child fails, the parent flow fails. A `[block.save_to]` key the child never set fails the flow.
- `explicit_only` skips tag-less discovery (`golem run`, `golem run <dir>`); a matching `--tag` or a direct path still runs it.
- Set `app_lifecycle = "manual"` in the child so it keeps the parent's running app.
````

</details>

<details><summary><code>help("flow", "reuse")</code> · Reuse: Subflow vs Mixin vs Fixture · 1011 chars</summary>

Source: [src/test-structure/50-reuse.md](src/test-structure/50-reuse.md)

```text
## Reuse: Subflow vs Mixin vs Fixture

Three ways to share pieces across flows, by what they contain:

| Concept | File / location | Contains | Reused via | Use when |
|---|---|---|---|---|
| **flow** | `x.test.toml` | `[flow]` + `[[block]]` | — (top-level unit) | a complete scenario |
| **subflow** | `x.test.toml`, usually `explicit_only = true` | a full `[flow]` | `run_flow` on a `[[block]]`; `[block.save_to]` propagates results back | reusing a whole scenario as a child (e.g. `login`) |
| **mixin** | `__mixins__/x.toml` | `[[step]]` only (no flow/block/vars) | `load_mixin` (help("act", "load_mixin")) action; steps inline into the block, per-call `vars` | reusing a step fragment that runs inside the caller's block state |
| **fixture** | `__fixtures__/x.toml` | `[vars]` only | `load_fixture` (help("act", "load_fixture")) action; access as `${alias.key}` | reusing test **data** |

`__mixins__/` and `__fixtures__/` are excluded from flow discovery, so their files never run as tests on their own.
```

</details>

<details><summary><code>help("flow", "lifecycle")</code> · Lifecycle: Setup &amp; Teardown · 1055 chars</summary>

Source: [src/test-structure/60-lifecycle.md](src/test-structure/60-lifecycle.md)

```text
## Lifecycle: Setup & Teardown

**There is no `[[setup]]` block.** A flow's setup is implicit and happens automatically before the first block:

1. **build and install** — once per app and device across the suite, cached (see App Install (help("install"))).
2. **app_lifecycle** — per flow, at flow start:
   - `reset` (default) — stop every app in `[[flow.apps]]`, then launch the first. Guarantees fresh state.
   - `launch` — launch the first app only if not already running. Preserves state.
   - `manual` — do nothing; the flow (or its parent) owns the app. `--start <block>` forces this.

Any additional setup you need (e.g. creating a user) is just normal steps, or a mixin (help("flow", "reuse")) if shared.

**Subflows** never re-build or re-install (that layer isn't re-entered for a `run_flow` child), but the child **does** re-run `app_lifecycle` with *its own* setting — which is why reusable subflows set `app_lifecycle = "manual"` to inherit the parent's running app.

Cleanup after the flow belongs in Teardown (help("flow", "teardown")).
```

</details>

<details><summary><code>help("flow", "teardown")</code> · Teardown · 763 chars</summary>

Source: [src/test-structure/70-teardown.md](src/test-structure/70-teardown.md)

````text
## Teardown

Teardown blocks run after the flow completes, regardless of pass/fail — running **even when the flow fails** is the point: it cleans up external state (test data, created users) that a failed run would otherwise leak. Failures in teardown don't affect the test result (they surface as `Teardown:` warnings on the report). Teardown runs before the automatic device-state reset (dark mode, mocked location, recording), so it still sees the app as the flow left it.

```toml
[[teardown]]
steps = [
  { action = "screenshot", path = "/tmp/final.png" },
  { action = "stop", app = "app" },
]
```

A teardown step can be a `load_mixin` (help("act", "load_mixin")), for example to share a logout sequence between flows.

Skip teardown with `--no-teardown`.
````

</details>

<details><summary><code>help("flow", "data-driven")</code> · Data-Driven Tests · 1050 chars</summary>

Source: [src/test-structure/80-data-driven.md](src/test-structure/80-data-driven.md)

````text
## Data-Driven Tests

A `[[data]]` table holds the rows, and a block iterates them with
`for_each = "data"`. The block runs once per row, and each row's fields are
read under the `${_each.<field>}` prefix:

```toml
[[data]]
user = "alice"

[[data]]
user = "bob"

[[block]]
for_each = "data"
steps = [
  { action = "type", on_text = "Search", input = "${_each.user}" },
  { action = "assert_visible", on_text = "${_each.user}" },
]
```

Only the `for_each` block repeats — surrounding blocks run once, and the
repeating block re-enters per row. An empty `[[data]]` table runs the block zero times.

Iteration is **block-level only**: rows parameterise steps inside a flow, not
whole flows. The block re-enters without relaunching the app, so a row that
leaves the app somewhere new is the next row's starting state — put anything
that must be reset into the block's own steps. To run a whole scenario per
case — each with a fresh app launch and its own pass/fail line — write it as
its own flow, or as a subflow invoked with different `[block.vars]`.
````

</details>

<details><summary><code>help("flow", "variables")</code> · Variables · 1404 chars</summary>

Source: [src/test-structure/90-variables.md](src/test-structure/90-variables.md)

````text
## Variables

Set variables with `--var NAME=value` on the CLI, in `[flow.vars]`, from data rows, with a step's `save_to`, or from fixtures. Reference them as `${name}`:

```toml
[flow.vars]
base_url = "https://staging.example.com"

[[block]]
steps = [
  { action = "read", on_right_of = "Status:", save_to = "current_status" },
  { action = "bash", run = "echo ${current_status}", save_to = "result" },
]
```

When the CLI, `golem.toml` and the flow set the same name, see Project config (help("flow", "project-config")) for which one wins.

### Built-in variables

golem reserves the `_` prefix and fills these in per step:

| Variable | Value |
|---|---|
| `_device` | Device name (`Pixel 9`) |
| `_os` | OS major version (`34`) |
| `_platform` | `android` / `ios` |
| `_type` | Device type (`phone`, `tablet`) |
| `_udid` | Device UDID |
| `_app` | App registry name of the step's app |
| `_hardware` | `virtual` on a sim/emulator, `real` on a physical device — same vocabulary as the `hardware` device constraint |
| `_loop` | 0-based count of times the current block has been entered |
| `_perf` | Last perf snapshot (object — see Performance Monitoring (help("flow", "performance-monitoring"))) |

`_hardware`, `_loop` and `_perf` are also readable by a branch condition
(`[[block.branch]] if_var = "_hardware", equals = "real"`); the device and app
builtins resolve in `${…}` interpolation only.
````

</details>

<details><summary><code>help("flow", "secrets")</code> · Secrets · 1969 chars</summary>

Source: [src/test-structure/95-secrets.md](src/test-structure/95-secrets.md)

````text
## Secrets

golem shows variable values in its output: the step lines, the reports, the logs and the MCP results. Tag a value as secret where it enters the run, and golem replaces the value with its reference everywhere it writes. For example, `t:"staging-password"` shows as `t:"${qa.password}"`.

Tag a value in one of these places:

- **A fixture.** A root `secret` list names the keys of `[vars]` that are secret. See `load_fixture` (help("act", "load_fixture")).
- **A step with `save_to`.** `secret = true` tags the whole saved value. `secret = ["token", "user.refresh_token"]` tags only those paths, for example the token of a test user that an HTTP step creates. If a path is not in the saved value, the step passes with the warning WP463, because the value under the real path is not masked.
- **`create_inbox`.** The inbox password (`pass`) is always secret.

To keep a secret out of every file, get it from the environment or a secret manager with a step:

```toml
{ action = "bash", run = "op read op://qa/password", save_to = "pw", secret = true }
```

golem masks the value, not the variable. A string made from a secret, such as `"Bearer ${user.token}"`, shows as `Bearer ${user.token}`, and a response that sends the token back is masked too. golem masks a secret of any length. A short secret, such as a PIN, also masks the same text elsewhere in the output: the output is harder to read, but the secret does not show. For an object such as the result of `run`, name the paths: `secret = true` also masks fields such as `exit_code`.

The limits:

- golem cannot mask pixels. A screenshot or a screen recording shows a secret that the app displays. Password fields usually show dots.
- golem cannot mask a changed form of a value: base64, URL encoding, a hash, or a value split across lines.
- Masking hides a value. It does not stop a step from using the value: a step can still send it with `bash` or `post_http`.
- Use test accounts, not real users.
````

</details>

<details><summary><code>help("flow", "fake-data")</code> · Fake Data Generators · 471 chars</summary>

Source: [src/test-structure/100-fake-data.md](src/test-structure/100-fake-data.md)

````text
## Fake Data Generators

`${fake:…}` generates random but valid test data: `email`, `password`, `uuid`, `number`, `one_of`, `sentence`, `timestamp`, `phone`, and the structured `person`, `address` and `credit_card`. `--seed <N>` replays the same values.

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
```

**See Fake Data Generators (help("fake"))** for every generator, its parameters and fields.
````

</details>

<details><summary><code>help("flow", "multi-app")</code> · Multi-App Flows · 537 chars</summary>

Source: [src/test-structure/110-multi-app.md](src/test-structure/110-multi-app.md)

````text
## Multi-App Flows

Test interactions across multiple apps:

```toml
[[flow.apps]]
name = "app"
bundle = "com.example.main"

[[flow.apps]]
name = "companion"
bundle = "com.example.companion"

[[block]]
steps = [
  { action = "tap", on_text = "+" },
  { action = "launch", app = "companion" },
  { action = "assert_visible", on_text = "Shared Data" },
  { action = "launch", app = "app" },
  { action = "stop", app = "companion" },
]
```

`launch` brings an app to foreground without restarting it. Use `restart = true` for a cold start.
````

</details>

<details><summary><code>help("flow", "devices")</code> · Devices · 1509 chars · + 4 items</summary>

Source: [src/test-structure/120-devices/00-intro.llm.md](src/test-structure/120-devices/00-intro.llm.md) (LLM text)

````text
## Devices

Most flows need no device settings: with no `[[flow.apps.devices]]`, the flow runs on whatever is booted (both platforms if both are booted). Set constraints only when the flow must run on a specific OS, shape or device.

```toml
[[flow.apps.devices]]
os = ["ios:latest", "android:34"]   # "ios:26", "ios:18.6", "ios:17+" (minimum), "ios:latest", "ios:latest:2"
type = "phone"                      # "phone" | "tablet"
hardware = "virtual"                # "virtual" | "real" | ["virtual", "real"]; omitted = either, virtual preferred
# name = "iPhone 15"                # exact device name
```

- Each `[[flow.apps.devices]]` entry is one combination that must run. An array inside one entry is an axis: every value must run somewhere.
- `[flow.options] coverage`: `smart` (default) ticks every axis value, using more devices when free; `min` uses the fewest devices that tick every value; `full` runs every combination; `one` stops after the first successful run.
- Use `hardware = "virtual"` for flows with `push_notification`.
- `[flow.options] create_if_missing = true` creates a simulator/emulator when none matches. It needs an `os` with a platform, and errors for `hardware = "real"` or `name`.

Items (help("flow", item)):
- coverage: How multi-valued device axes expand into runs: smart, min and more
- hardware: hardware = "virtual" or "real", or either
- pinning: name = "…": pin one device by its display name
- auto-boot: golem boots a matching shut-down device; headless on Android
````

<details><summary><code>help("flow", "coverage")</code> · Coverage Strategies · 2379 chars</summary>

Source: [src/test-structure/120-devices/10-coverage.md](src/test-structure/120-devices/10-coverage.md)

````text
### Coverage Strategies

`coverage` in Flow Options (help("flow", "flow/options")) controls how multi-valued `[[flow.apps.devices]]` axes expand into FlowRuns.

| Strategy | Behaviour |
|---|---|
| `smart` (default) | Ticks every axis value, and uses more devices when they are free. |
| `min` | Fewest devices that tick every axis value. Every planned run runs. |
| `full` | Cartesian product — one FlowRun per (os × type × …) combination. Use when every combo needs independent validation. |
| `one` | The first successful run ends the group. Local smoke testing. Tolerates underspec (`ios:latest:2` with only one version available). |

**Two ways to write device constraints**, with different meanings:

*Multi-block form — pinned tuples.* Each `[[flow.apps.devices]]` is an independent combination that must run.

```toml
[[flow.apps.devices]]
os = "ios:26"
type = "tablet"

[[flow.apps.devices]]
os = "android:34"
type = "phone"
```

This guarantees **both specific combinations**: an iPad v26 AND an Android phone v34.

*Single-block array form — independent axes.* Each axis value is a coverage point; Golem ticks every value but doesn't care how the combos fall out.

```toml
[[flow.apps.devices]]
os = ["ios:26", "android:34"]
type = ["tablet", "phone"]
```

This guarantees **every axis value runs somewhere**. Under `smart`/`min` two devices cover all four boxes — could be iPad v26 + Android phone v34, or iPhone v26 + Android tablet v34. Under `full` it emits four fully-pinned combinations.

**When the forms are equivalent.** If each block has at most one multi-valued axis (typically when `type` is absent or single-valued and identical across all blocks), the two forms produce the same boxes:

```toml
# Multi-block
[[flow.apps.devices]]
os = "ios:latest"
type = "phone"
[[flow.apps.devices]]
os = "android:latest"
type = "phone"

# Array form (equivalent — recommended for compactness)
[[flow.apps.devices]]
os = ["ios:latest", "android:latest"]
type = "phone"
```

Both emit two fully-pinned boxes `{ios, latest, phone}` + `{android, latest, phone}` under every strategy. Prefer the array form when it captures the same intent.

**No `[[flow.apps.devices]]` block at all.** Golem runs on whatever platform is currently booted (both if both are booted), on a device of either shape — a simulator is preferred when one is free. Fails fast if nothing is booted.
````

</details>

<details><summary><code>help("flow", "hardware")</code> · Hardware Axis (virtual / real) · 1578 chars</summary>

Source: [src/test-structure/120-devices/20-hardware.md](src/test-structure/120-devices/20-hardware.md)

````text
### Hardware Axis (virtual / real)

```toml
[[flow.apps.devices]]
# (hardware omitted)                # default: either shape, virtual preferred

[[flow.apps.devices]]
hardware = "virtual"                # explicit: virtual-only

[[flow.apps.devices]]
hardware = "real"                   # physical device required

[[flow.apps.devices]]
hardware = ["virtual", "real"]      # coverage axis — both tick boxes emitted
```

Omitting `hardware` means **either shape is acceptable**, and golem prefers a virtual one: a free booted simulator/emulator wins over a connected phone. A physical device is used only when no virtual device is free and already booted, so a plugged-in phone is usable without editing the flow.

Say `hardware = "virtual"` when the flow cannot run on hardware — `push_notification` is the standard case, and it has a lint that says so.

Under `coverage = "one"` / `"smart"`, `hardware = ["virtual", "real"]` degrades gracefully: the virtual box usually succeeds first, and the physical box is skipped. To *insist* on physical, use `hardware = "real"` on its own.

`create_if_missing` is a `[flow.options]` (or `golem.toml` `[options]`) key, default `false`. When no booted or shut-down device matches a slot, `true` makes golem create and boot a simulator/emulator from the slot's `os` and `type` (a phone if `type` is unset); `false` fails the slot with "No … devices found". It needs an `os` that names a platform. When no matching real device is connected, `hardware = "real"` with `create_if_missing = true` errors: golem cannot create physical hardware.
````

</details>

<details><summary><code>help("flow", "pinning")</code> · Pinning a Specific Device by Name · 387 chars</summary>

Source: [src/test-structure/120-devices/30-pinning.md](src/test-structure/120-devices/30-pinning.md)

````text
### Pinning a Specific Device by Name

```toml
[[flow.apps.devices]]
name = "iPhone 15"
```

`name` pins an exact device display name (as shown by `golem devices` / `xcrun simctl list` / `adb devices -l`). Use this when you have a customised simulator or a specific physical device the flow must target.

With `create_if_missing`, an unknown name errors instead of creating a simulator.
````

</details>

<details><summary><code>help("flow", "auto-boot")</code> · Auto-Boot Behaviour · 402 chars</summary>

Source: [src/test-structure/120-devices/40-auto-boot.md](src/test-structure/120-devices/40-auto-boot.md)

```text
### Auto-Boot Behaviour

When no booted device matches a slot but a matching simulator/emulator is shut down, golem boots it and waits until it is ready.

Emulators that golem boots on Android run headless — no window. To watch a device, boot it yourself before the run (golem reuses a booted device instead of starting another), or on iOS open `Simulator.app`, which shows the simulators golem boots.
```

</details>

</details>

<details><summary><code>help("flow", "project-config")</code> · Project config (golem.toml) · 1544 chars</summary>

Source: [src/test-structure/130-project-config.md](src/test-structure/130-project-config.md)

````text
## Project config (`golem.toml`)

`golem.toml` sits at the project root (golem walks up from the working
directory to find it) and holds the defaults every flow inherits. A flow
always wins over the project for anything it states itself.

```toml
[vars]                      # referenced in any flow as ${base_url}
base_url = "https://staging.example.com"

[options]                   # defaults for every flow's [flow.options]
step_timeout = 8000
record = true
max_device_wait = "30m"     # queue-wait cap; --max-device-wait beats it

[[apps]]                    # app registry — flows reference by name
name = "app"
bundle = "com.example.myapp"
install_script = "scripts/install.sh"

[[teardown]]                # appended to every flow's own teardown
steps = [
  { action = "bash", run = "scripts/cleanup.sh" },
]

[device_settings]           # OS-level tweaks applied once per device session
android = { "secure.long_press_timeout" = "400" }
```

| Section | Merge rule |
|---|---|
| `[vars]` | Flow `[flow.vars]` of the same name wins |
| `[options]` | Per field: a flow's `[flow.options]` value wins, others fall through |
| `[[apps]]` | Flows inherit `bundle` / `install_script` / `install_timeout_ms` / `devices` by app name |
| `[[teardown]]` | Runs after the flow's own teardown, and runs even when the flow failed |
| `[device_settings]` | Applied to the device before any flow runs — not a flow-level concept |

CLI flags beat both: `--var` overrides a project or flow var, and the
recording flags override `record` at every level.
````

</details>

</details>


### install

<details><summary><code>help("install")</code> · App Install · 1363 chars · + 9 items, 8 sub-items</summary>

Source: [src/app-install/00-intro.md](src/app-install/00-intro.md)

```text
# App Install

By default golem assumes the app under test is already installed on the target device. For fresh simulators, CI pipelines, or teams that want per-test builds, you can supply an install script that golem runs before each flow.

golem runs the script with four arguments: `$1` platform, `$2` device id, `$3` bundle id and `$4` mode (see Script contract (help("install", "script-contract"))). `golem install-script` writes one from a template. Without a terminal, `golem install-script --print <framework>` writes a template to stdout.

Items (help("install", item)):
- quick-start: golem install-script, and --print / --template without a terminal
- apps-registry: Declare each app once in golem.toml [[apps]]; flows inherit it
- install-script-forms: One script for both platforms, or one for each platform
- script-contract: The arguments: $1 platform, $2 device id, $3 bundle id, $4 mode
- install-env-and-profiles: install_env values and --profile app definitions
- install-cache: golem installs once per app and device, and when it installs again
- cache-flags: --rebuild forces an install; --no-build skips it
- no-install-script: Without install_script, golem launches the installed app
- frameworks: The templates golem install-script writes, one per framework — native-ios, native-android, tauri, expo, capacitor, kmp, nativescript, flutter
```

<details><summary><code>help("install", "quick-start")</code> · Quick start · 1510 chars</summary>

Source: [src/app-install/10-quick-start.md](src/app-install/10-quick-start.md)

````text
## Quick start

```bash
golem install-script      # interactive: choose framework, answer prompts
```

The scaffold writes `scripts/install-<app>-<platform>.sh` (for native iOS/Android) or `scripts/install-<app>.sh` (for cross-platform frameworks like Tauri and Expo) and can auto-update `golem.toml`.

**Without a terminal** (for example from an agent), use one of these:

```bash
golem install-script --list           # the frameworks that have a template
golem install-script --print expo     # write the Expo template to stdout, then fill in its {{VALUES}}
golem install-script --template expo --app requester --path apps/requester-app
```

With `--template`, the command asks nothing. Each value that no flag gives is the default its prompt would show, detected from the project as the prompt detects it. golem adds the script to the app's `[[apps]]` entry in `golem.toml`, if there is one. `--app` is the app's name in `[[apps]]` (default `app`). `--path` is the app's project directory. For `native-ios`, `--path` is the `.xcodeproj` or `.xcworkspace`, or a directory that has one. The templates are in Frameworks the scaffold supports (help("install", "frameworks")).

**Monorepos:** a repository can have many apps. Run the command once for each app, with its own `--app` and `--path`, or loop over the `[[apps]]` of `golem.toml`:

```bash
golem install-script --template expo --app requester --path apps/requester-app
golem install-script --template expo --app provider  --path apps/provider-app
```
````

</details>

<details><summary><code>help("install", "apps-registry")</code> · Project-level [[apps]] registry · 638 chars</summary>

Source: [src/app-install/20-apps-registry.md](src/app-install/20-apps-registry.md)

````text
## Project-level `[[apps]]` registry

Declare each app once in `golem.toml`. Flows reference by `name` and inherit everything else:

```toml
# golem.toml
[[apps]]
name = "app"
bundle = "com.example.app"
install_script = { ios = "scripts/install-app-ios.sh", android = "scripts/install-app-android.sh" }
install_timeout_ms = 900000   # optional override (default 600000 = 10 min)
```

```toml
# flow.test.toml
[[flow.apps]]
name = "app"       # inherits bundle + install_script + install_timeout_ms from [[apps]]

[[flow.apps.devices]]
os = "ios:latest"
type = "phone"
```

Flow-level fields override project-level ones when both are set.
````

</details>

<details><summary><code>help("install", "install-script-forms")</code> · install_script forms · 355 chars</summary>

Source: [src/app-install/30-install-script-forms.md](src/app-install/30-install-script-forms.md)

````text
## `install_script` forms

Either a single path (cross-platform):

```toml
install_script = "scripts/install.sh"
```

Or a platform-keyed table (native separate iOS + Android builds):

```toml
install_script = { ios = "scripts/install-ios.sh", android = "scripts/install-android.sh" }
```

Golem picks the right entry for the target platform at run time.
````

</details>

<details><summary><code>help("install", "script-contract")</code> · Script contract · 1354 chars</summary>

Source: [src/app-install/40-script-contract.md](src/app-install/40-script-contract.md)

````text
## Script contract

Golem invokes the script from the project root with three positional args:

```text
script.sh <platform> <device_udid> <bundle_id>
```

- `platform`: `"ios"` or `"android"`
- `device_udid`: simulator UDID / emulator serial / physical device identifier
- `bundle_id`: from `[[apps]]` or `[[flow.apps]]`

Exit 0 = success, golem launches the app. Nonzero = flow fails with the script's stderr captured; subsequent flows using the same `(device, bundle)` pair are skipped.

Stdout is discarded. Stderr is streamed live via the event system and shows up in:

- Human output: `[install <app>]` prefix
- `results.json` / `results.toon` / `results.xml`: under the top-level `installs` list, with success/duration/exit_code/error

Scripts also support a 4th `"install-only"` arg for manual dev-iteration (skip build, reuse previous artifact). Golem currently always passes empty; this is reserved for a future build-once-install-many optimisation.

### Environment

The script inherits golem's environment, plus:

- **`GOLEM_REBUILD`** — `"1"` when the run passed `--rebuild`, else `"0"`. A golem-injected builtin (the `GOLEM_` prefix is reserved for these). Lets a script that owns a remote/build cache force a fresh build; scripts that ignore it are unaffected.
- **Your `install_env`** — any key/values you declare on the app (see below).
````

</details>

<details><summary><code>help("install", "install-env-and-profiles")</code> · Parameterizing installs: install_env + profiles · 2672 chars</summary>

Source: [src/app-install/50-install-env-and-profiles.md](src/app-install/50-install-env-and-profiles.md)

````text
## Parameterizing installs: `install_env` + profiles

Two composable knobs let one script cover staging/sandbox/prod, Debug/Release, or local-vs-cloud builds without hand-editing scripts.

### `install_env` — inject environment values

Declare env vars on an app; golem injects them into the install script's process:

```toml
[[apps]]
name = "app"
bundle = "com.example.app"
install_script = "scripts/install.sh"
install_env = { APP_ENV = "staging", SANDBOX_ID = "${sandbox_id}" }
```

Values interpolate `${var}` through the var engine **at install time**, so dynamic values flow from `--var`:

```bash
golem run flows/ --var sandbox_id=12345      # SANDBOX_ID becomes "12345"
```

Injection is the existing `--var` — there is no separate `--install-env` flag. Because install runs *before* any flow step, only install-time scopes resolve: CLI `--var`, project/flow vars, and the builtins `_platform` / `_udid` / `_app`. A `${...}` referencing a flow-step, device-, or per-row `each` var **fails the install loudly** (naming the var) rather than silently interpolating to empty and building the wrong artifact.

### Profiles — select app *definitions* by `--profile`

The same app `name` may appear in multiple `[[apps]]` / `[[flow.apps]]` entries, disambiguated by a `profile` field. A `profile`-less entry is the catch-all default. `golem run --profile <name>` picks, per app, the matching entry — else falls back to the catch-all.

```toml
# Default (bare `golem run`): local dev build.
[[apps]]
name = "app-e"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"

# `golem run --profile eas`: same script, cloud build (via install_env).
[[apps]]
name = "app-e"
profile = "eas"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"
install_env = { EXPO_BUILD_MODE = "eas", EAS_PROFILE = "preview" }
```

- Entries key by `(name, profile)`. A flow entry field-merges over the project entry of the **same** key (flow wins); different profile keys never bleed into each other. Profile entries are self-contained — restate shared fields (e.g. `bundle`), nothing inherits across profiles.
- Falling back to the catch-all is by design — golem notes it, it is not an error. It **hard-errors** only when a referenced app has neither a `(name, profile)` entry nor a catch-all. A `--profile` that matches nothing across every app is flagged as a likely typo.
- The script never sees the profile name — `--profile` only selects which entry (hence which `install_script` / `install_env`) is active. This is the CI/local lever: a `ci` profile can use a release/standalone build while the default uses a dev build, without duplicating flows.
````

</details>

<details><summary><code>help("install", "install-cache")</code> · Install cache · 2689 chars</summary>

Source: [src/app-install/60-install-cache.md](src/app-install/60-install-cache.md)

```text
## Install cache

Two layers of caching, both transparent in the default mode:

**In-memory (per suite)** — keyed on `(device_udid, bundle_id)`. Within a single `golem run`, the script runs at most once per combination.

- `Succeeded` → subsequent flows on the same device skip the script and go straight to launch
- `FailedScript` / `FailedNoScript` → subsequent flows using that combo are **skipped** with a clear reason (no repeated retries on broken setups)

**Persistent (cross-run)** — `.golem/install-cache.json`, keyed `(udid, bundle)` → `{ fingerprint, device_install_time, installed_version, installed_at }`. Subsequent `golem run` invocations skip both build AND install when **all three** integrity gates pass:

1. **Device-present** — device reports the bundle as installed (`xcrun simctl get_app_container` / `adb shell pm path`)
2. **Install-time matches** — device's bundle mtime / `lastUpdateTime` matches the cached `device_install_time`. Catches external reinstalls (Xcode "Run", manual `simctl install`)
3. **Fingerprint matches** — current source fingerprint equals the cached one. Tier 1: `git rev-parse HEAD` + sha1 of `git status --porcelain`. Tier 2 (non-git): content hash of the project tree honouring `.gitignore`

Any gate failing → cache miss, normal build+install runs, fresh entry written.

**On hit** the live stream prints `skipped (cache hit)` — terse. A hit always means **all three gates passed**; the source-fingerprint identity is implied (you almost always know what state your tree is in already, so listing it on every hit is noise).

**On miss** the stream prints a specific reason so you can see *why* a build was triggered. Examples:

- `cache miss on iPhone 17 — source fingerprint changed (git:abc → git:def)` — you committed / edited code (label shows clean→dirty or rev→rev movement)
- `cache miss on iPhone 17 — device install-time differs (... — external reinstall?)` — Xcode "Run", manual `simctl install`, or another tool replaced the binary
- `cache miss on iPhone 17 — bundle no longer installed on device` — sim was reset / app was uninstalled
- `cache miss on iPhone 17 — fingerprint unavailable (no git, no readable source tree)` — neither tier could compute a fingerprint (extremely rare)

The "no prior cache entry" case (first-time install on a fresh checkout) is silent — that's the normal path on a cold cache, not a cache invalidation worth flagging.

Where the label *does* render (in fingerprint-changed misses): clean trees show just the rev (`git:abc1234`); dirty trees include a 4-char porcelain-hash suffix (`git:abc1234+0a1b`) so two dirty trees with the same commit but different uncommitted edits render distinctly.
```

</details>

<details><summary><code>help("install", "cache-flags")</code> · Cache flags: --rebuild and --no-build · 1383 chars</summary>

Source: [src/app-install/70-cache-flags.md](src/app-install/70-cache-flags.md)

````text
## Cache flags: `--rebuild` and `--no-build`

Two flags control cache behaviour. Default (no flag) = strict mode: read cache, skip on full gate match, write after build.

**`--rebuild`** — force a fresh build for this run. Bypasses cache reads so every `(device, bundle)` is rebuilt + reinstalled. The cache is **still written** after a successful build, so the next run benefits. Use when:

- A flaky build produced a bad binary and you want to start clean
- You suspect the cache is wrong but don't want to manually delete it
- You're verifying a CI build matches what local develops

```bash
golem run flows/ --rebuild
```

**`--no-build`** — skip build+install entirely. The cache is **not consulted and not written**. For each `(device, bundle)`:

- Device has the bundle → flow runs against the existing binary
- Device missing the bundle → flow fails immediately with `--no-build: <bundle> not installed on <device>; drop --no-build or install manually`

Use when:

- Iterating on flow files only — the binary hasn't changed and you want to skip even the ~150ms cache check
- You built manually via Xcode / Android Studio and want golem to test against that

```bash
golem run flows/ --no-build
```

**Both passed** — `--no-build` wins; golem emits a warning. The two intents are mutually exclusive (force rebuild vs trust device), so passing both is almost always a mistake.
````

</details>

<details><summary><code>help("install", "no-install-script")</code> · When no install_script is configured · 870 chars</summary>

Source: [src/app-install/80-no-install-script.md](src/app-install/80-no-install-script.md)

```text
## When no `install_script` is configured

If an app has no `install_script` field in `golem.toml` or its flow file, golem behaves like a permanent `--no-build` for that app: no install runs, the flow goes straight to `launch`. The two paths converge on the success case but differ on the failure mode:

| Scenario | App present | App absent |
|---|---|---|
| **No `install_script` in TOML** | launches, runs flow | `launch` errors at runtime, cache marks `FailedNoScript`, subsequent flows skip with that reason |
| **`--no-build` flag** | mark `Succeeded` upfront, runs flow | flow fails immediately with an actionable hint to drop the flag |

So if your project has no install scripts anywhere, `--no-build` is redundant — golem already assumes the apps are preinstalled. The flag earns its keep when scripts *are* configured and you want to bypass them temporarily.
```

</details>

<details><summary><code>help("install", "frameworks")</code> · Frameworks the scaffold supports · 833 chars · + 8 items</summary>

Source: [src/app-install/90-frameworks/00-intro.md](src/app-install/90-frameworks/00-intro.md)

```text
## Frameworks the scaffold supports

`golem install-script` has a template for each of these frameworks. `golem install-script --print <framework>` writes one to stdout. A script is plain bash: change it freely after you scaffold it. For another framework, write the script by hand, to the script contract (help("install", "script-contract")).

Items (help("install", item)):
- native-ios: xcodebuild, then simctl or devicectl install
- native-android: Gradle assemble, then adb install
- tauri: tauri ios/android build, then install
- expo: Expo prebuild and a local Release build, or EAS; golem run --dev
- capacitor: Web build, cap sync, then a native build and install
- kmp: Kotlin Multiplatform with Compose Multiplatform
- nativescript: ns build, then install the artifact
- flutter: flutter build, then adb or simctl install
```

<details><summary><code>help("install", "native-ios")</code> · native-ios · 632 chars</summary>

Source: [src/app-install/90-frameworks/10-native-ios.md](src/app-install/90-frameworks/10-native-ios.md)

```text
### native-ios

xcodebuild + `xcrun simctl install` (simulator) / `xcrun devicectl` or `ios-deploy` (physical). Reads these variables from `install_env` or the shell:
- `CONFIGURATION` and `XCODE_SCHEME`: override the values written into the script.
- `XCCONFIG`: an `.xcconfig` path, passed as `-xcconfig`.
- `DEVELOPMENT_TEAM`: your Apple team ID, for a physical device. The script passes it as a build setting with `-allowProvisioningUpdates`. If a physical-device build fails, the error names this variable. **The physical-device path is not yet validated (#59).**

The script fails if the `.app` is older than the build start.
```

</details>

<details><summary><code>help("install", "native-android")</code> · native-android · 642 chars</summary>

Source: [src/app-install/90-frameworks/20-native-android.md](src/app-install/90-frameworks/20-native-android.md)

```text
### native-android

`./gradlew :<module>:assemble<Flavor><BuildType>`, then `adb -s <serial> install` of the APK that build wrote. Reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: the Gradle build type, `debug` by default. A release build needs a `signingConfig`, because Android installs only signed APKs.
- `FLAVOR`: the product flavor. With more than one flavor dimension, use the full flavor name, for example `freeProd`.

The script fails if the APK is older than the build start. If `aapt2` is found under `ANDROID_HOME/build-tools`, the script also fails when the APK's `applicationId` is not the app's `bundle`.
```

</details>

<details><summary><code>help("install", "tauri")</code> · tauri · 815 chars</summary>

Source: [src/app-install/90-frameworks/30-tauri.md](src/app-install/90-frameworks/30-tauri.md)

```text
### tauri

`tauri ios build` / `tauri android build` then install. Detects package manager from lockfiles (`npm` / `yarn` / `pnpm` / `bun` / `cargo tauri`). Reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release`. A release build on Android needs a `signingConfig` in `src-tauri/gen/android`, because Android installs only signed APKs. Without one, the script stops with an error that names the signing problem.
- `TAURI_BUILD_CONFIG`: passed to `tauri … build --config`. It is JSON, or the path to a config file that tauri merges over `tauri.conf.json`.
- `TAURI_BUILD_FEATURES`: passed to `tauri … build --features`, comma-separated.

The script installs only an artifact that the current build wrote. It fails if the `.app` or the APK is older than the build start.
```

</details>

<details><summary><code>help("install", "expo")</code> · expo · 2936 chars</summary>

Source: [src/app-install/90-frameworks/40-expo.md](src/app-install/90-frameworks/40-expo.md)

```text
### expo

Expo / React Native. One cross-platform script honouring `EXPO_BUILD_MODE`:
- `local` (default): `expo prebuild` + a **Release** native build (embeds the JS bundle → runs offline, no Metro), then `simctl` / `adb` install. Fully local — no Expo account, works on a simulator/emulator with no signing account.
- `eas`: build in the cloud via EAS, download the artifact, install it. Reuses the latest finished build unless `GOLEM_REBUILD=1`. Needs `EXPO_TOKEN`. **Written but unverified in golem's own CI** (no account) — validate against a real project before relying on it.

Detects package manager from lockfiles; auto-discovers the iOS scheme from the generated app project when not set.

Expo (managed) only — it relies on `expo prebuild` to generate the native projects. **Bare React Native** (a `@react-native-community/cli` project with no `expo` dependency) commits its own `ios/` + `android/` and has no `prebuild`; use **native-ios** + **native-android** for those.

**Iterating on JS: `golem run --dev`.** A Release build embeds the bundle, so
every JS change costs a full native rebuild. Build a Debug build once
(`npx expo run:ios` / `npx expo run:android` — these also install and launch
it), leave `npx expo start` running, and then `golem run --dev` skips
build+install entirely: the per-flow relaunch re-fetches the current bundle
from the dev server, so a JS edit is picked up with no rebuild and no
reinstall. golem waits for the dev server and fails with `H503` if it never
answers, rather than letting the resulting red error screen read as a
missing selector. Local iteration only — a CI run should test the artifact
it ships. Simulators and emulators only; a physical device also needs
`adb reverse tcp:8081 tcp:8081` or a reachable LAN host.

**Broken JS is reported as `A501`, not as a failing test.** With a bundle
that can't build, every selector times out and the run would otherwise
blame the flow. Two things prevent that:

- Before any flow runs, `--dev` asks the dev server for the app's bundle. A
  build failure stops the run and quotes the error (`SyntaxError: App.tsx:
  Unexpected token (9:15)`). This is the only signal available on iOS,
  where React Native's error overlay renders nothing golem can see.
- During a run, a step that fails against a visible error overlay is
  reported as `A501` with the overlay's own message instead of as a missing
  selector. This covers both of React Native's overlays: the native redbox,
  shown when the bundle never ran (Android — the same failures leave an iOS
  simulator blank), and the JS LogBox, shown when the app mounted and then
  threw, which renders on both platforms and also reports the source
  position (`boom at App.tsx (35:43)`).

  A LogBox minimised to its badge, or open on a console warning or console
  error, leaves a working app underneath and is deliberately left alone —
  a flow failing there is failing on its own merits.
```

</details>

<details><summary><code>help("install", "capacitor")</code> · capacitor · 1374 chars</summary>

Source: [src/app-install/90-frameworks/50-capacitor.md](src/app-install/90-frameworks/50-capacitor.md)

```text
### capacitor

Capacitor / Ionic. One cross-platform script:
1. Install JS dependencies when the lockfile or `package.json` changed.
2. Run the web build (`npm run build` by default, empty for none). The script fails if no file in `webDir` was written by this build, because `cap sync` would otherwise ship the previous bundle.
3. Run `cap sync <platform>`.
4. Build and install with Gradle and `adb` (Android) or `xcodebuild` and `simctl` (iOS). These are the same steps as the native templates, with the same freshness checks.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release`. It selects the Gradle build type and the Xcode configuration (`Debug` / `Release`).
- `FLAVOR`, `XCCONFIG`, `DEVELOPMENT_TEAM`, `DERIVED_DATA`: as for native-android and native-ios.

The `android/` and `ios/` projects are source in a Capacitor app, so the script never creates them. If one is missing, the script fails and names `cap add <platform>`. The iOS build uses `ios/App/App.xcworkspace` when it exists (CocoaPods), else `ios/App/App.xcodeproj` (Swift Package Manager, the Capacitor 8 default), with the scheme `App`.

**Release builds:** golem reads the page through the web inspector, which Capacitor turns off in release builds. To test a release build, set `webContentsDebuggingEnabled: true` in the Capacitor config.
```

</details>

<details><summary><code>help("install", "kmp")</code> · kmp · 2280 chars</summary>

Source: [src/app-install/90-frameworks/60-kmp.md](src/app-install/90-frameworks/60-kmp.md)

```text
### kmp

Kotlin Multiplatform with Compose Multiplatform. One cross-platform script:
- Android builds the app module (`composeApp` or `androidApp`) with `assemble<Flavor><BuildType>` and installs the APK with `adb`. These are the same steps and checks as native-android.
- iOS builds the Xcode project in the app directory (`iosApp`). A CocoaPods integration (`kotlin("native.cocoapods")`) builds through its `.xcworkspace` and runs `pod install` first when `Pods/Manifest.lock` differs from `Podfile.lock`.
- **The script checks that the built `.app` holds the Kotlin code before it installs it.** `xcodebuild` succeeds without the Kotlin framework when the project lacks the step that builds it, and the app then fails at launch. If the check fails, make sure of two things: the iOS target in the shared module's `build.gradle.kts` declares `binaries.framework { … }`, and the Xcode target has a Run Script phase, before Compile Sources, that runs `./gradlew :<shared module>:embedAndSignAppleFrameworkForXcode`.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release`. It selects the Gradle build type and the Xcode configuration (`Debug` / `Release`).
- `FLAVOR`, `XCODE_SCHEME`, `XCCONFIG`, `DEVELOPMENT_TEAM`, `DERIVED_DATA`: as for native-android and native-ios.
- `ORG_GRADLE_PROJECT_<name>`: Gradle reads each of these as `-P<name>=<value>` itself, for the Android build and for the iOS framework build.

**Compose Multiplatform 1.8.0 or later is required on iOS.** From 1.8.0 the Compose accessibility tree syncs to iOS automatically, and that tree is what golem reads.

**Selectors on a Compose screen:**
- Target buttons by their visible text, and give them a `contentDescription` for accessibility. On Android, a `contentDescription` is a separate accessibility label (`on = { accessibility_label = … }`). On iOS, Compose Multiplatform merges it into the button's label, as `"Increment, +"`.
- Do not put a `contentDescription` on text whose value a step checks. On iOS the description replaces the visible text, so a counter labelled `"count"` reads as `"count"`, not as its value.

For element types, `testTag`, bounds and merged semantics, see Selectors → Canvas-rendered UI (help("selectors", "canvas")).
```

</details>

<details><summary><code>help("install", "nativescript")</code> · nativescript · 1525 chars</summary>

Source: [src/app-install/90-frameworks/70-nativescript.md](src/app-install/90-frameworks/70-nativescript.md)

```text
### nativescript

NativeScript 8+. One cross-platform script: it installs JS dependencies when they changed, runs `ns build <platform>`, then installs the artifact that build wrote with `simctl` / `devicectl` or `adb`. It does not use `ns build --copy-to`, which covers only the device `.ipa` and has been broken for simulator builds. The iOS `.app` is looked up under both `platforms/ios/build/` and `platforms/ios/Build/`, because CLI versions differ.

**Host needs:** the NativeScript CLI, as a `nativescript` devDependency (run with `npx ns`) or a global `ns`, and CocoaPods + Ruby for iOS.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release` (adds `--release`).
- `NS_BUILD_ARGS`: extra `ns build` flags, for example `--env.*` bundler flags, or the `--key-store-*` flags that an Android release build needs for signing.
- `DEVELOPMENT_TEAM` (`--team-id`) or `PROVISION` (`--provision`): signing for a physical iOS device, which builds with `--for-device`.

**Selectors:** NativeScript renders real native views, so text selectors work on both platforms. `automationText` (and `testID`) set the accessibility *identifier*. On iOS golem matches it with `accessibility_label`. On Android it is only a view tag, not a `resource-id`, so the accessibility tree does not carry it and golem cannot see it. To give an Android view a label golem can match, set `accessibilityLabel` (Android's `contentDescription`). On iOS that label replaces the view's visible text.
```

</details>

<details><summary><code>help("install", "flutter")</code> · flutter · 1804 chars</summary>

Source: [src/app-install/90-frameworks/80-flutter.md](src/app-install/90-frameworks/80-flutter.md)

```text
### flutter

Flutter 3.19+. One cross-platform script: it runs `flutter build apk` or `flutter build ios`, then installs the artifact that build wrote with `adb` or `simctl` / `devicectl`. It does not use `flutter install`, which uninstalls the app first and so wipes its data and granted permissions on every install. The scaffold detects a `pubspec.yaml` with an `sdk: flutter` dependency, and suggests `fvm flutter` when the project pins its SDK with FVM.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default), `profile` or `release`. A simulator runs `debug` only. A physical iOS device runs `profile` or `release` only, because a debug build starts there only under the debugger.
- `FLAVOR`: the Flutter flavor (`--flavor`), that is, a Gradle product flavor and an Xcode scheme.
- `DART_DEFINES`: space-separated `KEY=VALUE` pairs, each passed as `--dart-define`. A value cannot contain a space. For such values, use `DART_DEFINE_FILE`.
- `DART_DEFINE_FILE`: a `.json` or `.env` file for `--dart-define-from-file`.
- `FLUTTER_BUILD_ARGS`: extra `flutter build` flags, for example `--obfuscate --split-debug-info=<dir>`. Do not pass `--split-per-abi`: the script installs one APK.
- `DEVELOPMENT_TEAM`: the Apple team ID for a physical iOS device. Flutter hands it to xcodebuild as `FLUTTER_XCODE_DEVELOPMENT_TEAM`.

**Selectors:** Flutter draws into one canvas view and copies its semantics tree into the platform accessibility tree. Text selectors work on both platforms. `Semantics(identifier:)` is the identifier that `on_accessibility_label` matches. `MergeSemantics` joins its children's text into one node and removes the children. For element types, merged semantics and bounds, see Selectors → Canvas-rendered UI (help("selectors", "canvas")).
```

</details>

</details>

</details>


### fake

<details><summary><code>help("fake")</code> · Fake Data Generators · 1257 chars · + 5 items, 11 sub-items</summary>

Source: [src/fake-data/00-intro.md](src/fake-data/00-intro.md)

````text
# Fake Data Generators

Generate realistic test data with `${fake:…}`. A generator works anywhere `${…}`
interpolation does — in a variable declaration or inline in a step field:

```toml
[flow.vars]
email = "${fake:email}"
user  = "${fake:person(country=JP)}"
addr  = "${fake:address(country=GB)}"
card  = "${fake:credit_card(brand=visa)}"

[[block]]
steps = [
  { action = "type", on_text = "Email", input = "${fake:email}" },
  { action = "type", on_text = "City",  input = "${addr.city}" },
]
```

A **scalar** generator (e.g. `email`) resolves straight to a string. An
**object** generator (`person`, `address`, `credit_card`, `timestamp`) exposes several fields
— read one with dot notation (`${user.given}`, `${addr.city}`, `${card.number}`);
using the bare object where a string is expected is an error.

Items (help("fake", item)):
- seeds: --seed replays the same values; dates anchor to the seed
- all-generators: Table of every generator and what it returns
- simple-generators: Each returns one string — email, password, uuid, number, one_of, sentence, phone
- structured-generators: These return objects; read fields with `.field` — person, address, credit_card, timestamp
- cross-references: A generator argument can read an earlier variable
````

<details><summary><code>help("fake", "seeds")</code> · Determinism and seeds · 576 chars</summary>

Source: [src/fake-data/10-seeds.llm.md](src/fake-data/10-seeds.llm.md) (LLM text)

```text
## Determinism and seeds

- The same seed gives the same `${fake:…}` values, so assertions on generated values replay exactly.
- Set it with `golem run <flow> --seed <N>`. Without `--seed`, values are fresh each run, and the report shows the seed used; pass it back with `--seed` to replay that run. In an MCP session: `session_open(seed = <N>)`. A `[flow] seed` in the file has no effect.
- Dates (`fake:timestamp`, card expiry) are relative to an anchor carried in the seed: a run without `--seed` anchors at the current time; a small seed like `--seed 42` anchors at 2020.
```

</details>

<details><summary><code>help("fake", "all-generators")</code> · All generators · 1088 chars</summary>

Source: [src/fake-data/20-all-generators.md](src/fake-data/20-all-generators.md)

```text
## All generators

Everything `${fake:…}` supports, at a glance:

| Generator | Returns | Summary |
|-----------|---------|---------|
| `${fake:email}` | scalar | Random email address |
| `${fake:password}` | scalar | Random password |
| `${fake:uuid}` | scalar | UUID v4 |
| `${fake:number}` | scalar | Random integer |
| `${fake:one_of}` | scalar | Random pick from a caller-supplied set |
| `${fake:sentence}` | scalar | One-sentence filler; `language=` takes an ISO 639-1 code (114 languages), lorem default |
| `${fake:phone}` | scalar | Country-formatted phone number |
| `${fake:person}` | object | Names across scripts: `.given` / `.family` / `.reading` / `.ascii` / per-script branches |
| `${fake:address}` | object | `.street` / `.city` / `.state` / `.postcode` / `.country` / `.country_code` / `.lat` / `.lon` |
| `${fake:credit_card}` | object | Luhn-valid card: `.number` / `.expiry` / `.cvv` / `.brand` / `.provider` / `.status` / `.threeds` |
| `${fake:timestamp}` | object | Date/time: `.datetime` / `.date` / `.time` / `.year` / `.month` / `.day`; window + age params |
```

</details>

<details><summary><code>help("fake", "simple-generators")</code> · Simple generators · 365 chars · + 7 items</summary>

Source: [src/fake-data/30-simple-generators/00-intro.md](src/fake-data/30-simple-generators/00-intro.md)

```text
## Simple generators

Each returns one string. Defaults are shown for each.

Items (help("fake", item)):
- email: Random email address
- password: Random password
- uuid: UUID v4
- number: Random integer in a range
- one_of: Random pick from values you give
- sentence: One sentence of filler, in a language you choose
- phone: Phone number formatted for a country
```

<details><summary><code>help("fake", "email")</code> · email · 825 chars</summary>

Source: [src/fake-data/30-simple-generators/10-email.md](src/fake-data/30-simple-generators/10-email.md)

```text
### email

`${fake:email}` → `<random>@example.com`.

| Param | Effect |
|-------|--------|
| `domain` | Replace `example.com` — `fake:email(domain=acme.test)` → `<random>@acme.test` |
| `prefix` | Prepend to the random local part — `fake:email(prefix=qa_)` → `qa_<random>@example.com` |

**Real-inbox / plus-addressing.** Put a `+` in `prefix` and your mailbox's domain
in `domain` to keep every address unique while delivering to one real mailbox:
`fake:email(prefix=alice+, domain=gmail.com)` → `alice+<random>@gmail.com`, all
delivered to `alice@gmail.com`. Combine with the
`await_email` (help("act", "await_email")) action to
verify mail end-to-end.

**Disposable real inbox.** `${fake:email}` has no inbox behind it; to receive
mail at a throwaway address, use the
`create_inbox` (help("act", "create_inbox"))
action.
```

</details>

<details><summary><code>help("fake", "password")</code> · password · 213 chars</summary>

Source: [src/fake-data/30-simple-generators/20-password.md](src/fake-data/30-simple-generators/20-password.md)

```text
### password

`${fake:password}` → a random password. Params: `length` (default 12);
`symbols` (default `true` — set `symbols=false` for letters + digits only).
Example: `fake:password(length=20, symbols=false)`.
```

</details>

<details><summary><code>help("fake", "uuid")</code> · uuid · 94 chars</summary>

Source: [src/fake-data/30-simple-generators/30-uuid.md](src/fake-data/30-simple-generators/30-uuid.md)

```text
### uuid

`${fake:uuid}` → a v4 UUID, e.g. `f47ac10b-58cc-4372-a567-0e02b2c3d479`. No params.
```

</details>

<details><summary><code>help("fake", "number")</code> · number · 163 chars</summary>

Source: [src/fake-data/30-simple-generators/40-number.md](src/fake-data/30-simple-generators/40-number.md)

```text
### number

`${fake:number}` → a random integer (as a string). Params: `min` (default 0) and
`max` (default 100), inclusive. Example: `fake:number(min=1, max=6)`.
```

</details>

<details><summary><code>help("fake", "one_of")</code> · one_of · 319 chars</summary>

Source: [src/fake-data/30-simple-generators/50-one_of.md](src/fake-data/30-simple-generators/50-one_of.md)

```text
### one_of

`${fake:one_of(free|pro|enterprise)}` → one value picked at random from the set.
Separate choices with `|` or `,`: `fake:one_of(yes|no)`, `fake:one_of(JP, US, GB)`.
Spaces around a choice are trimmed and empty choices are dropped. A choice
cannot contain `|`, `,` or `=`. Seeded like every other generator.
```

</details>

<details><summary><code>help("fake", "sentence")</code> · sentence · 820 chars</summary>

Source: [src/fake-data/30-simple-generators/60-sentence.md](src/fake-data/30-simple-generators/60-sentence.md)

```text
### sentence

`${fake:sentence}` → a short one-sentence filler string for textarea / bio /
comment / description fields. Default is **lorem ipsum**.

| Param | Effect |
|-------|--------|
| `language` | ISO 639-1 code — generate in that language. **110+ languages** supported (every code in the bundled sentence corpus — en, es, zh, hi, ar, fr, pt, ru, ja, de, ko, sw, …). Omitted → lorem ipsum. An unsupported code is an error. |

`fake:sentence(language=ja)` → `古いゴーレムが石を砕く。`,
`fake:sentence(language=fr)` → `Le gardien garde la pierre.` Sentences are
golem-myth-themed (clay, stone, guardians), script-correct
(CJK/Thai join without spaces, Arabic/Hebrew are right-to-left), and
seed-reproducible.

Grammar is deliberately simplified, and some less-common-script languages are
machine-authored pending native review.
```

</details>

<details><summary><code>help("fake", "phone")</code> · phone · 572 chars</summary>

Source: [src/fake-data/30-simple-generators/70-phone.md](src/fake-data/30-simple-generators/70-phone.md)

```text
### phone

`${fake:phone}` → a plausibly-formatted phone number.

| Param | Effect |
|-------|--------|
| `country` | ISO code; uses that country's dialing format — `fake:phone(country=JP)` → `+81-…`. Unset → a random country's format. An **unrecognised** code is an error (a typo shouldn't silently yield another country) |
| `format` | Explicit template where `#` becomes a random digit — `fake:phone(format=+1 (###) ###-####)`. Takes precedence over `country` |

Chain it off an address to keep them consistent:
`phone = "${fake:phone(country=${addr.country_code})}"`.
```

</details>

</details>

<details><summary><code>help("fake", "structured-generators")</code> · Structured generators · 355 chars · + 4 items</summary>

Source: [src/fake-data/40-structured-generators/00-intro.md](src/fake-data/40-structured-generators/00-intro.md)

```text
## Structured generators

These return objects; read fields with `.field`.

Items (help("fake", item)):
- person: Name parts in many scripts: .given, .family, .ascii
- address: .street, .city, .postcode, .country and more
- credit_card: Luhn-valid card: .number, .expiry, .cvv, .brand
- timestamp: Date and time in a window or by age: .date, .time, .year
```

<details><summary><code>help("fake", "person")</code> · person · 2749 chars</summary>

Source: [src/fake-data/40-structured-generators/10-person.llm.md](src/fake-data/40-structured-generators/10-person.llm.md) (LLM text)

````text
### person

`${fake:person}` is an object of name parts. Read a field; the bare object is an error in a string.

- `country` is the **form's** country: it sets the script the form expects, not where the name comes from. A name in the country's script stays native; any other name falls back to ASCII (`country=JP`: `田中` stays, `Dupont` stays `Dupont`; `country=DE`: `Müller` stays, `André` becomes `Andre`).
- Names include characters that break naive validators: `O'Brien`, `Jean-Pierre`, `ß`.
- There is no full name. Build it the way the form wants: `"${p.given} ${p.family}"` (Western), `"${p.family}${p.given}"` (JP/KR/CN, no space).
- Every field is a string; a part that does not apply is `""`.

| Field | Value |
|-------|-------|
| `given` / `family` | What a local types into the form's given/family field |
| `reading.given` / `reading.family` | Reading (furigana): katakana for `country=JP`, hangul for `country=KR`; `""` elsewhere |
| `ascii.given` / `ascii.family` | Pure-ASCII romanisation |
| `<rep>.given` / `<rep>.family` | One representation, regardless of country: `native`, `ascii`, `kana`, `hiragana`, `katakana`, `hangul`, `cyrillic`, `hebrew`, `arabic`, `hanja` |

```toml
[flow.vars]
p = "${fake:person(country=JP)}"
# ${p.family} -> 田中   ${p.reading.given} -> ユキ   ${p.ascii.family} -> Tanaka
```

| Param | Sets | Example |
|-------|------|---------|
| `country` | A preset for `name`, `reading` and `local` | `country=JP` |
| `name` | The chain for `given`/`family`: the first non-empty representation wins | `name=[local, ascii]` |
| `reading` | The chain for `reading` | `reading=[katakana]` |
| `local` | The characters counted as native, for the `local` representation (a set, not a chain) | `local=[ascii, diacritics_fr]` |

- Precedence: an explicit `name`/`reading`/`local` beats the `country` preset, which beats the default (no country: `name=[native]`, no reading, `local` accepts everything).
- Representation tokens (`name`, `reading`): `native`, `local`, `ascii`, `kana`, `hiragana`, `katakana`, `hangul`, `cyrillic`, `hebrew`, `arabic`, `hanja`. `local` is the native name if every character is in the `local` set, else `""`.
- Repertoire tokens (`local`): `ascii`, `kanji`, `hiragana`, `katakana`, `hangul`, `hanzi`, `hanja`, `cyrillic`, `hebrew`, `arabic`, `devanagari`, `thai`, `diacritics_<lang>` for `de fr es pt it sv ga mi pl lt nl`. An unknown token is an error.
- Supported `country` codes (case-insensitive): AE, AU, BE, BR, CA, CN, DE, EG, ES, FR, GB, IE, IL, IN, JP, KR, LT, MX, NL, NZ, PL, RU, SE, SG, TH, US, ZA. Every preset's `name` is `[local, ascii]`. JP (`reading=[katakana]`) and KR (`reading=[hangul]`) also fill `reading`. An unlisted code is not an error; it behaves like no country.
````

</details>

<details><summary><code>help("fake", "address")</code> · address · 2028 chars</summary>

Source: [src/fake-data/40-structured-generators/20-address.md](src/fake-data/40-structured-generators/20-address.md)

```text
### address

`${fake:address}` returns a coherent address for one place — all fields come from
the same city, so they stay consistent. There are no standalone city, postcode or
street generators: read the field you need (`${fake:address.city}`).

| Param | Effect |
|-------|--------|
| `country` | ISO 3166-1 alpha-2 code, case-insensitive. Unset → a random country. An unrecognised code is an error. Supported: AE, AU, BE, BR, CA, CN, DE, EG, ES, FR, GB, IE, IL, IN, JP, KR, LT, MX, NL, NZ, PL, RU, SE, SG, TH, US, ZA. |
| `state` | Limit to one state, by its native or romanised name (`東京` or `Tokyo`), case-insensitive. |
| `region` | Limit to the states tagged with this region, case-insensitive — e.g. `Kansai` / `Kanto` (JP), `New England` (US), `Scotland` (GB). The tags for each country are the `region_tags` in `data/geo/<code>.json`. |

`state` and `region` need `country`; without it they are ignored. A filter that
matches no state is an error.

The text fields default to the place's **native script**; an `ascii` sub-object
carries the romanised forms — exactly like `${fake:person}` (native default,
`.ascii` branch). Romanisations are ASCII folds of the native, never English
exonyms (`Bayern`, not "Bavaria").

#### Fields

| Field | `${fake:address}` (native) | `${fake:address.ascii.*}` |
|-------|---------|---------|
| `street` | 北一条西５ | 5 Kita 1-jo Nishi |
| `city` | 札幌市 | Sapporo |
| `state` | 北海道 | Hokkaido |
| `postcode` | 060-0001 | 060-0001 |
| `country` | 日本 | Nihon |
| `country_code` | JP | *(top-level only)* |
| `lat` | 43.0621 | *(top-level only)* |
| `lon` | 141.3544 | *(top-level only)* |

`country_code` / `lat` / `lon` are script-neutral and appear only at the top
level (not in `ascii`). The native and ascii streets share the same house
number, rendered in the script's own numerals (full-width for JP). `lat` / `lon`
are **approximate** — the chosen city's centre, not the street point. A Latin
country (e.g. GB) reads identically in both: native `42 Baker Street` ==
`ascii.street`.
```

</details>

<details><summary><code>help("fake", "credit_card")</code> · credit_card · 1719 chars</summary>

Source: [src/fake-data/40-structured-generators/30-credit_card.md](src/fake-data/40-structured-generators/30-credit_card.md)

```text
### credit_card

`${fake:credit_card}` generates a Luhn-valid card. Params:

- `brand` — `visa` / `mastercard` / `amex` / `discover` (sets the number prefix
  and length).
- `provider` — `stripe` / `adyen` / `square` / … selects that payment provider's
  published **test-card** set, so `number` and `status` match what the provider's
  sandbox expects. Without it, a generic Luhn-valid card is produced. An unknown
  provider is an error, and the error message lists the valid providers (one per
  file in `data/cards/`, e.g. `pay_jp`, `checkout_com`).
- `status` — the simulated outcome (see below).

#### Fields

| Field | Example |
|-------|---------|
| `number` | 4532015112830366 |
| `expiry` | 03/28 |
| `cvv` | 123 |
| `brand` | visa |
| `provider` | `stripe`; `""` without a provider |
| `status` | `""` for an approved card without a provider; otherwise the status |
| `threeds` | `true` — present only when `status` is `threeds` (with a provider, any status starting with `threeds`) |

Some provider test cards carry extra fields the provider's form needs, such as
`name`, `postal_code`, `otp` or `pin`.

#### Statuses

Without a provider, an approved card has an **empty** `status` (`""`). To
simulate a failure, pass `status=`; without a provider the options are
`approved`, `declined:invalid_number`, `declined:expired`,
`declined:invalid_cvv`, `threeds`. Any other status needs a provider.

With a provider, `status` defaults to `approved`, and the valid statuses are the
keys of that provider's `statuses` table in `data/cards/<provider>.json` (Stripe,
for example, adds `declined:insufficient_funds`, `declined:lost`, `fraud`,
`threeds:failed`). A status the provider has no test card for is an error.
```

</details>

<details><summary><code>help("fake", "timestamp")</code> · timestamp · 1351 chars</summary>

Source: [src/fake-data/40-structured-generators/40-timestamp.md](src/fake-data/40-structured-generators/40-timestamp.md)

```text
### timestamp

`${fake:timestamp}` returns a date/time object — read the field the form needs:

| Field | Example | Notes |
|-------|---------|-------|
| `.datetime` | `2025-09-14T08:21:00+00:00` | ISO 8601 |
| `.date` | `2025-09-14` | `YYYY-MM-DD` |
| `.time` | `08:21` | `HH:MM` |
| `.year` / `.month` / `.day` | `2025` / `09` / `14` | zero-padded parts — for forms with separate date inputs (e.g. date of birth) |

Address a field directly: `${fake:timestamp.date}` (the bare object errors in a
string context, like the other object generators).

Anchored on the run's reference instant (see
Determinism and seeds (help("fake", "seeds"))) and **seed-reproducible**.
The date is drawn from a window measured **in whole years relative to the
anchor** — positive years are in the past, **negative years in the future**; by
default that is the last year.

| Param | Effect |
|-------|--------|
| `max_years` | The far edge (default 1). `fake:timestamp(max_years=5).date` → within the last 5 years |
| `min_years` | The near edge (default 0, i.e. the anchor) |

A **date of birth** is a window pushed into the past — set both edges:
`fake:timestamp(min_years=18, max_years=90).date` → someone aged 18–90 at the
anchor. A **future** date (a licence/subscription expiry up to 5 years out) uses
negatives: `fake:timestamp(min_years=-5, max_years=0).date`.
```

</details>

</details>

<details><summary><code>help("fake", "cross-references")</code> · Cross-references · 246 chars</summary>

Source: [src/fake-data/50-cross-references.md](src/fake-data/50-cross-references.md)

````text
## Cross-references

Later generators can reference earlier variables:

```toml
[flow.vars]
addr   = "${fake:address(country=JP)}"
phone  = "${fake:phone(country=${addr.country_code})}"
person = "${fake:person(country=${addr.country_code})}"
```
````

</details>

</details>


### codes

<details><summary><code>help("codes")</code> · Error Codes · 859 chars · + 1 item</summary>

Source: [src/error-codes/00-intro.llm.md](src/error-codes/00-intro.llm.md) (LLM text)

```text
# Error Codes

A failed or warned step carries a code such as `EF404`: `<severity><domain><number>`.

- Severity: `E` for a failure, `W` for a warning (the step set `if_fail = "warn"`).
- Domain, the party that most likely owns the fix:
  - `F`: flow at run time; the test or the app is wrong.
  - `P`: the flow file, its fields or the suite config.
  - `A`: the app's build, install or launch.
  - `D`: the device, its boot or the companion.
  - `H`: the host: tools, ports, the golem daemon.
  - `S`: the user's `golem mcp` policy refused the action (MCP only).
  - `X`: golem did not classify the error.
- Number: the cause, the same for `E` and `W`.

help("codes", "EF404") gives one code's meaning and fix. help("codes", "registry") lists every code, its meaning and its fix.

Items (help("codes", item)):
- registry: Every code with its meaning and fix
```

<details><summary><code>help("codes", "registry")</code> · Registry · 5740 chars</summary>

Source: [src/error-codes/10-registry.md](src/error-codes/10-registry.md)

```text
## Registry

| Code | Meaning | Fix |
|------|---------|-----|
| `F400` | A `fail` step ran. | Check the branch or condition that reached the `fail` step. |
| `F404` | No visible element matched the selector before the timeout. | Fix the selector, add `auto_scroll = true`, or wait for the screen first. |
| `F405` | The element exists, but scrolling did not bring it into view. | Set `within` to the scroll container that holds the element. |
| `F408` | The step ran past its timeout. | Raise the step's `timeout`; if many steps time out, check the host and device. |
| `F409` | `assert_not_visible`: the element is still on screen. | Wait for the screen change first, or report the app bug. |
| `F412` | The alert or text did not match the expected value. | Fix the expected value, or report the app bug. |
| `F417` | An alert is shown, but golem could not press its button. | Name a button that the alert has. |
| `F424` | A `bash`, `run` or HTTP step failed, or `await_email` found no match. | Read the output or response; fix the script, endpoint or email filter. |
| `F504` | The flow ran past `max_runtime`. | Raise `max_runtime`, or find the slow or looping blocks. |
| `F508` | The flow ran more than `max_steps` steps, usually in a loop. | Fix the `next` or branch loop, or raise `max_steps`. |
| `F509` | A block ran more times than its `max_iterations`. | Fix the loop that re-enters the block, or raise its `max_iterations`. |
| `X000` | golem did not classify this error. | Read the message, and report it to the golem maintainers. |
| `S001` | The user's `golem mcp` policy denies this action. | Tell the user; the user changes the policy with `golem mcp-config`. |
| `S002` | This action needs the user's permission. | Ask the user; if the user agrees, call act again with the same step. |
| `P400` | The `action` is not a known action. | Fix the spelling; see the actions reference. |
| `P404` | A block, subflow or fixture that the flow names does not exist. | Fix the name or path in `next`, `goto`, `run_flow` or `load_fixture`. |
| `P422` | A required field is missing or not valid. | Add or fix the field that the message names. |
| `P450` | A `${…}` reference has bad syntax or type, or names an unknown generator. | Fix the variable reference or the generator name. |
| `P460` | The flow or mixin file is not valid TOML, or does not fit the schema. | Fix the file at the line that the message names. |
| `P461` | No device can satisfy the flow's device constraints. | Relax the device constraints, or add a device that matches. |
| `P462` | `add_media` got a file that is not a supported image or video. | Use a supported image or video file. |
| `P463` | A `secret` path names no key of the value the step saved. | Fix the path in `secret`, so the value under it is masked. |
| `A403` | The install script path is outside the project. | Keep `install_script` inside the project directory. |
| `A404` | The install script or the app bundle does not exist. | Fix the `install_script` or bundle path, or build the app first. |
| `A408` | The app install ran past its timeout. | Make the install faster, or raise `install_timeout_ms`. |
| `A500` | The install script exited with an error. | Read the script output, and fix the build or the install. |
| `A501` | The app shows a React Native error overlay instead of its UI. | Fix the JavaScript error that the overlay names. |
| `A502` | golem could not read the app's state after the install. | Check that the app installed; install it again. |
| `A503` | The app did not launch or stop. | Check the bundle id and the app's crash log (`app_logs`). |
| `D404` | No device matches. | Boot or connect a device; `golem devices` lists them. |
| `D408` | The device did not finish booting in time. | Boot the simulator or emulator by hand once, and check its image. |
| `D409` | Every matching device stayed busy past `--max-device-wait`. | Free a device, add a device, or raise `--max-device-wait`. |
| `D500` | golem could not create the simulator or emulator. | Install the runtime or system image that the device needs. |
| `D502` | golem could not talk to the webview inspector. | Make the webview debuggable, then try again. |
| `D503` | The companion runs but is stuck on a call. | Try again; if it repeats, restart the device or the app. |
| `D504` | The companion did not register with golem in time. | Check that the companion installed and launched; install it again. |
| `D505` | The companion refused the connection; it is not running. | Try again; if it repeats, reduce the host load or restart the device. |
| `D506` | The companion stopped after each restart, so golem gave up. | Restart the device, and check the host's free memory and CPU. |
| `D507` | The companion connection closed during a request. | Check whether the step had its effect, then try again. |
| `D520` | A device driver operation failed. | Read the message; for `press`, use a button that the device has. |
| `H404` | A tool or file that golem needs is missing. | Install what the message names; `golem doctor` checks the host. |
| `H429` | golem has no free ports. | Run fewer devices at once, or free ports. |
| `H502` | The connection to the golem daemon failed. | Restart the daemon, and check `GOLEM_SOCKET`. |
| `H424` | `browse_*` steps found no Chrome or Chromium. | Install Chrome, or set `$CHROME` to the browser binary. |
| `H501` | This golem build has no browser support. | Use a default build (without `--no-default-features`). |
| `H503` | `--dev`: no Expo or Metro dev server answers. | Start the dev server (`npx expo start`), or set `--dev-port`. |
| `H505` | The browser lacks a feature that the flow needs (WebMCP). | Upgrade the browser; `golem doctor` checks it. |
```

</details>

</details>

