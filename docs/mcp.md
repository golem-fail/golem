# MCP Server

*Another mind speaks the words; the golem does the work.*

← [Back to README](../README.md) · See also [CLI Reference](cli-reference.md#golem-mcp) · [Actions Reference](actions-reference.md)

`golem mcp` is an [MCP](https://modelcontextprotocol.io) server. An LLM client, such as Claude Code, Codex, Cursor or Claude Desktop, uses it to drive an iOS or Android device one step at a time. Use it for two tasks:

- **Write a flow.** The LLM runs steps on a live device. Each step that passes goes into a flow draft. The LLM then exports the draft as a `.test.toml` file that `golem run` runs.
- **Debug an app.** The LLM taps through the app, reads the screen, takes screenshots and reads the app's device log, then reports what it found.

The tool list and each tool's arguments are in the [CLI reference](cli-reference.md#golem-mcp). This page tells you how to set up a client and how a session works.

## Contents

- [Setup](#setup)
  - [Terminal agents](#terminal-agents): Claude Code, Codex CLI, Gemini CLI, OpenCode, GitHub Copilot CLI, Goose, Devin CLI
  - [Editors](#editors): Cursor, Cline, Continue, Zed, Windsurf
  - [Desktop apps](#desktop-apps): Claude Desktop, AnythingLLM, LibreChat
  - [Your own agent](#your-own-agent): Vercel AI SDK, PydanticAI, Spring AI
  - [Scripts](#scripts): mcpc
  - [Install channels](#install-channels)
  - [Timeouts](#timeouts)
  - [When the client drops the instructions](#when-the-client-drops-the-instructions)
- [The step notation](#the-step-notation)
- [The visible tree decides](#the-visible-tree-decides)
- [Sessions](#sessions)
  - [A session from a flow](#a-session-from-a-flow)
  - [Long operations](#long-operations)
- [The draft and step status](#the-draft-and-step-status)
  - [Edit the draft](#edit-the-draft)
  - [Blocks](#blocks)
  - [Run the draft again](#run-the-draft-again)
  - [Other draft tools](#other-draft-tools)
- [Example: write a new flow](#example-write-a-new-flow)
- [Example: edit a flow that exists](#example-edit-a-flow-that-exists)
- [Example: debug an app issue](#example-debug-an-app-issue)
- [Upgrades and troubleshooting](#upgrades-and-troubleshooting)

## Setup

Before you start, run `golem doctor`.

Each client starts the command `golem` with the argument `mcp` on the machine that runs your simulators and emulators. A client in the cloud, or in a container without access to those devices, cannot use golem.

`golem mcp --print-config <client>` prints a ready-made block with the absolute path of the `golem` that you ran. It knows `claude`, `codex`, `opencode`, `gemini`, `copilot`, `goose`, `zed`, `continue` and `desktop`. The other clients read one of these formats, as their sections say.

**Devices.** `session_open` uses a running simulator, emulator or connected device that fits. If none runs, golem boots one, as `golem run` does. Booting takes up to a few minutes, and the open answers `pending` meanwhile. Tell the LLM which device to use with the flow syntax, for example `os = "ios:26"` and `type = "tablet"`, or name one device with `device`. The [CLI reference](cli-reference.md#golem-mcp) has the rules.

**Apps that do not get your shell `PATH`.** An editor or desktop app that starts from the Dock or a launcher may not get your shell `PATH`. Then it cannot find `golem`, and golem cannot find `adb` or `xcrun`. In such a client, set `command` to the output of `which golem`, and set `PATH` and `ANDROID_HOME` in the server's `env` (see [Claude Desktop](#claude-desktop)). `--print-config desktop` and `--print-config zed` write these values for you.

### Terminal agents

#### Claude Code

Add the server for the current project:

```bash
claude mcp add golem -- golem mcp
```

- The default scope is `local`: this project, on your machine only.
- `--scope user` adds the server to all your projects.
- `--scope project` writes `.mcp.json` at the project root. Commit that file to share the server with your team:

  ```json
  { "mcpServers": { "golem": { "command": "golem", "args": ["mcp"] } } }
  ```

Check the server with `/mcp` in a Claude Code session, or with `claude mcp list`. Guide: [Claude Code MCP](https://code.claude.com/docs/en/mcp).

#### Codex CLI

```bash
codex mcp add golem -- golem mcp --project /path/to/your/project
```

The command writes this block to `~/.codex/config.toml`:

```toml
[mcp_servers.golem]
command = "golem"
args = ["mcp", "--project", "/path/to/your/project"]
```

Codex can start the server from any directory, so give `--project`. A trusted project can also hold the block in `.codex/config.toml`. Check the server with `codex mcp list`. Guide: [Codex MCP](https://developers.openai.com/codex/mcp).

#### Gemini CLI

```bash
gemini mcp add golem golem mcp
```

- The default scope is `project`: the command writes `.gemini/settings.json` in the project. Commit that file to share the server with your team.
- `--scope user` writes `~/.gemini/settings.json`. Then add `--project /path/to/your/project` to the server's arguments.

The block has the same form as the Claude Code block. Check the server with `/mcp` in a Gemini CLI session, or with `gemini mcp list`. `gemini mcp list` shows the server as connected only in a trusted folder. Guide: [Gemini CLI MCP servers](https://geminicli.com/docs/tools/mcp-server/).

#### OpenCode

OpenCode has no command that adds a server. Add the server to `opencode.json` at the project root, or to `~/.config/opencode/opencode.json` for all your projects:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "golem": { "type": "local", "command": ["golem", "mcp"], "enabled": true }
  }
}
```

In the global file, add `"--project", "/path/to/your/project"` to `command`. Check the server with `opencode mcp list`. Guide: [OpenCode MCP servers](https://opencode.ai/docs/mcp-servers/).

#### GitHub Copilot CLI

```bash
copilot mcp add golem --timeout 120000 -- golem mcp
```

The command writes `~/.copilot/mcp-config.json`. For one project, put the block in `.mcp.json`, or in `.github/mcp.json` to commit it:

```json
{ "mcpServers": { "golem": { "type": "stdio", "command": "golem", "args": ["mcp"], "tools": ["*"], "timeout": 120000 } } }
```

`timeout` (milliseconds) is Copilot's limit for one call. Check the server with `copilot mcp list`, or `/mcp` in a session. Guide: [Add MCP servers to Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers).

#### Goose

Run `goose configure`, then choose Add Extension and Command-line Extension, with the command `golem mcp --project /path/to/your/project`. Or add the block to `~/.config/goose/config.yaml` (`%APPDATA%\Block\goose\config\config.yaml` on Windows):

```yaml
extensions:
  golem:
    type: stdio
    name: golem
    enabled: true
    cmd: golem
    args: ["mcp", "--project", "/path/to/your/project"]
    envs: {}
    timeout: 300
```

Goose has no project scope, so give `--project`. To use golem in one session only, run `goose session --with-extension "golem mcp"`. In the Goose desktop app, use the absolute path of `golem`. Guide: [Using extensions](https://goose-docs.ai/docs/getting-started/using-extensions).

#### Devin CLI

```bash
devin mcp add golem -- golem mcp
```

The command writes `~/.config/devin/mcp_config.json`. For one project, put the Claude Code block in `.devin/mcp_config.json`, or in `.devin/mcp_config.local.json` to keep it out of git. Check the server with `devin mcp list`. Guide: [Devin CLI MCP](https://docs.devin.ai/cli/extensibility/mcp/configuration). Hosted Devin runs in Devin's own machine, not next to your devices, so it cannot use golem on your simulators.

### Editors

#### Cursor

Add the block to `.cursor/mcp.json` in the project, or to `~/.cursor/mcp.json` for all your projects. The Cursor CLI (`agent`) reads the same files.

```json
{ "mcpServers": { "golem": { "type": "stdio", "command": "/opt/homebrew/bin/golem", "args": ["mcp", "--project", "${workspaceFolder}"] } } }
```

Use the absolute path of `golem`. Check the server in Cursor's MCP settings, or with `agent mcp list`. Guide: [Cursor MCP](https://cursor.com/docs/context/mcp).

#### Cline

Open the MCP Servers panel, then Configure, then Configure MCP Servers. Add the block to `cline_mcp_settings.json`:

```json
{ "mcpServers": { "golem": { "command": "/opt/homebrew/bin/golem", "args": ["mcp", "--project", "/path/to/your/project"], "timeout": 600 } } }
```

Use the absolute path of `golem`. `timeout` is in seconds (default 60). The Cline CLI adds the server with `cline mcp add golem --yes -- golem mcp`. Guide: [Adding and configuring MCP servers](https://docs.cline.bot/mcp/adding-and-configuring-servers).

Cline does not pass a server's instructions to the model ([cline#14806](https://github.com/cline/cline/issues/14806)), so the model does not learn when to use golem. Add the instructions to `.clinerules`, as [When the client drops the instructions](#when-the-client-drops-the-instructions) shows.

#### Continue

Continue uses MCP servers in Agent mode only. Write the block to `.continue/mcpServers/golem.yaml` in the project:

```yaml
name: golem
version: 0.0.1
schema: v1
mcpServers:
  - name: golem
    type: stdio
    command: golem
    args: ["mcp", "--project", "/path/to/your/project"]
```

For all your projects, add the entry under `mcpServers:` in `~/.continue/config.yaml`. Continue also accepts a Claude Code JSON block copied into `.continue/mcpServers/`. Guide: [Continue MCP](https://docs.continue.dev/customize/deep-dives/mcp).

#### Zed

Add the server under `context_servers` in `.zed/settings.json` in the project, or in `~/.config/zed/settings.json`:

```json
{
  "context_servers": {
    "golem": {
      "command": "/opt/homebrew/bin/golem",
      "args": ["mcp", "--project", "/path/to/your/project"],
      "env": { "PATH": "/opt/homebrew/bin:/usr/bin:/bin", "ANDROID_HOME": "/Users/you/Library/Android/sdk" }
    }
  }
}
```

Check the server in Settings, AI, MCP Servers: a green dot means that it runs. Guide: [Zed MCP](https://zed.dev/docs/ai/mcp).

#### Windsurf (Devin Desktop)

Windsurf is now Devin Desktop. Add the Claude Desktop block, with the absolute path of `golem`, to `~/.config/devin/mcp_config.json`. Older Windsurf versions read `~/.codeium/windsurf/mcp_config.json`. Check the server in the Cascade panel, under the `…` menu, MCPs. Guide: [Cascade MCP](https://docs.devin.ai/desktop/cascade/mcp).

### Desktop apps

#### Claude Desktop

Add the block to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS), then restart the app.

```json
{
  "mcpServers": {
    "golem": {
      "command": "/opt/homebrew/bin/golem",
      "args": ["mcp", "--project", "/path/to/your/project"],
      "env": {
        "PATH": "/opt/homebrew/bin:/usr/bin:/bin:/Users/you/Library/Android/sdk/platform-tools",
        "ANDROID_HOME": "/Users/you/Library/Android/sdk"
      }
    }
  }
}
```

`golem mcp --print-config desktop` writes this block with your current `PATH` and `ANDROID_HOME`. Guide: [Connect to local MCP servers](https://modelcontextprotocol.io/docs/develop/connect-local-servers).

#### AnythingLLM

The desktop app reads `plugins/anythingllm_mcp_servers.json` in its storage folder: `~/Library/Application Support/anythingllm-desktop/storage` on macOS, `~/.config/anythingllm-desktop/storage` on Linux. The file appears when you open the Agent Skills page. Add the Claude Desktop block, then press refresh on that page. Call the tools with `@agent` in a chat. The Docker version of AnythingLLM cannot reach your devices. Guide: [AnythingLLM MCP](https://docs.anythingllm.com/mcp-compatibility/desktop).

#### LibreChat

LibreChat runs MCP servers on its own server, so golem works only with a native (not Docker) install on the machine with the devices. Add the server to `librechat.yaml`:

```yaml
mcpServers:
  golem:
    type: stdio
    command: golem
    args: ["mcp", "--project", "/path/to/your/project"]
    timeout: 120000
```

`timeout` (milliseconds) is LibreChat's limit for one call. Guide: [LibreChat MCP servers](https://www.librechat.ai/docs/configuration/librechat_yaml/object_structure/mcp_servers).

### Your own agent

An agent framework starts `golem mcp` as a stdio MCP server, as any client does. golem knows the client names that the Vercel AI SDK and Spring AI send, and answers within their limits (see [Timeouts](#timeouts)).

- **Vercel AI SDK** ([guide](https://ai-sdk.dev/docs/ai-sdk-core/mcp-tools)):

  ```ts
  import { createMCPClient } from '@ai-sdk/mcp';
  import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

  const golem = await createMCPClient({
    transport: new StdioClientTransport({ command: 'golem', args: ['mcp'] }),
  });
  const tools = await golem.tools();
  ```

- **PydanticAI** ([guide](https://pydantic.dev/docs/ai/mcp/client/)):

  ```python
  from fastmcp.client.transports import StdioTransport
  from pydantic_ai import Agent
  from pydantic_ai.mcp import MCPToolset

  golem = MCPToolset(StdioTransport(command='golem', args=['mcp']))
  agent = Agent('anthropic:claude-sonnet-5-5', toolsets=[golem])
  ```

- **Spring AI** ([guide](https://docs.spring.io/spring-ai/reference/api/mcp/mcp-client-boot-starter-docs.html)). The default `request-timeout` is 20 s. A longer one means fewer `pending` answers:

  ```yaml
  spring:
    ai:
      mcp:
        client:
          request-timeout: 120s
          stdio:
            connections:
              golem:
                command: golem
                args: [mcp]
  ```

### Scripts

[mcpc](https://github.com/apify/mcpc) calls MCP tools from a shell or a script, with no model. It reads the Claude Code block from `~/.mcpc/config.json` or `./mcpc.config.json`:

```bash
mcpc connect ./mcpc.config.json:golem @golem
mcpc @golem tools-list
```

`--timeout` (seconds, default 60) sets the limit for one call.

### Install channels

| Install | Command |
|---------|---------|
| brew or the install script | `golem mcp` |
| npm dev dependency | `npx golem mcp`, or the absolute path `node_modules/.bin/golem` with the argument `mcp` |

### Timeouts

A client stops waiting for one tool call after a limit. golem answers every call before its **soft timeout**. If the work is not done by then, the call answers `pending`, and the LLM calls `wait` for the result. See [Long operations](#long-operations).

golem sets the soft timeout from the client that connects, by the name in its MCP handshake: two thirds of the client's default limit, at most 120 s. A client that golem does not know gets 45 s. You do not need to set anything. golem writes its choice to stderr when the client connects, for example `golem mcp: client github-copilot-developer 1.0.62 · soft timeout 20s`.

| Client | Default limit for one call | golem's soft timeout | Client setting |
|--------|----------------------------|----------------------|----------------|
| Claude Code | none (27 h) | 120 s | `MCP_TOOL_TIMEOUT` (ms), or `"timeout"` in `.mcp.json` |
| Codex | 60 s in the docs, 300 s in the source | 40 s | `tool_timeout_sec` |
| Gemini CLI | 600 s | 120 s | `"timeout"` (ms) |
| OpenCode | 60 s | 40 s | `"timeout"` (ms) |
| Copilot CLI | 30 s in the docs, 180 s in the source | 20 s | `"timeout"` (ms) |
| Goose | 300 s | 120 s | `timeout` (s) |
| Cline, Continue, Zed, mcpc | 60 s | 40 s | per client, see its section |
| Claude Desktop (agent mode) | 60 s | 40 s | none |
| LibreChat | 30 s in the docs, 60 s in the source | 20 s | `timeout` (ms) |
| Spring AI | 20 s | 13 s | `spring.ai.mcp.client.request-timeout` |
| Vercel AI SDK, VS Code | none | 120 s | none |
| Cursor, Windsurf, Devin, others | unknown | 45 s | per client |

Where a client's docs and its source disagree, golem uses the lower limit. golem knows each client's default only, not a value you set. If you lower a client's limit below golem's soft timeout, or use a client that golem does not know and that waits less than 45 s, start the server with `--soft-timeout <secs>` below that limit. `--soft-timeout` always wins.

### When the client drops the instructions

golem sends short instructions when a client connects: what golem is, and when to use it. Most clients keep this text in the model's context, so the model considers golem for a mobile e2e task even before it looks at a tool. Each tool's description says how to use that tool.

Some clients drop the instructions, for example Cline. With such a client, give the model the same text in another way. Use the first way that your client supports:

1. **An Agent Skill.** Write the skill into the client's skills folder. For Claude Code:

   ```bash
   mkdir -p .claude/skills/golem
   golem mcp --print-skill > .claude/skills/golem/SKILL.md
   ```

   The skill's `description` tells the client when to use golem.
2. **`AGENTS.md`, or the client's rules file.** Add the plain text:

   ```bash
   golem mcp --print-instructions >> AGENTS.md
   ```

   For Cline, add the text to `.clinerules` instead.

Both commands print the text that the server sends, so the text stays the same after an upgrade. Run the command again after you upgrade golem.

## The step notation

A step is one TOML inline table on one line. It is the same text as a step in a flow file:

```toml
{ action = "tap", on_text = "Sign in" }
{ action = "type", on_text = "Email", input = "ada@example.test" }
{ action = "assert_visible", on_text = "Welcome" }
```

`act` runs one step. `probe` takes the same table without `action`, and shows what the selector matches.

Select by `on_text`, the text that a user reads. Use `on_accessibility_label` only when the test checks that label, or when the element has no text. When a step that selects by its label passes, and `on_text` with the element's text selects the same element, `act` adds a hint with that `on_text`.

`help` serves the docs one piece at a time, so the LLM reads only what it needs:

| Call | Answer |
|------|--------|
| `help()` | The topics: `act`, `selectors`, `flow`, `fake`, `codes` |
| `help("act")` | The step notation, the common actions, and each group of actions |
| `help("act", "browser")` | One group, as a list of its actions |
| `help("act", "tap")` | One action: its fields and examples |
| `help("flow", "branching")` | One section of the flow file docs |
| `help("codes", "EF404")` | One failure code: its meaning and its fix |

An item is the last part of a section's name (`tap`), its path (`interaction/tap`), or an action name in its heading (`post_http`). The text comes from the parts in [`docs/src`](src). Where a part has an `.llm.md` version, `help` gives that shorter text.

## The visible tree decides

golem tests as a person does. A step finds its element only in the visible tree: the elements on the screen, not the elements that are off the screen or hidden under other elements. `tree` returns the visible tree. `tree(full = true)` returns every element, marked as a hint. Use the full tree to find where to scroll, but never to decide that a step passed. `probe` lists off-screen matches as hints for the same reason.

## Sessions

A session holds one device, one app, the variables and a flow draft. Each MCP server has one session at a time.

1. `session_open` picks the device and the app. It can first run a flow (see below). `devices` lists every device in any state, with the port of a live companion.
2. `act`, `probe`, `tree`, `screenshot` and `app_logs` work on that device.
3. `session_close` ends the session and releases the device.

A session also ends in these cases:

| End | The flow's `[[teardown]]` runs |
|-----|------|
| `session_close` | yes, unless `teardown = false` |
| The client stops the server (stdin closes, or the process is killed) | no |
| No operation for `idle_timeout_s` (default 1800) | no |
| golem stops, for example after an upgrade | no |

Only an explicit `session_close` runs the teardown. A teardown can change the app or the device. golem does not do that when nobody asked for it.

### A session from a flow

`session_open(flow = "e2e/login.test.toml")` runs the flow first, as `golem run` would: install, apps, launch and steps. Then the session opens where the flow stops, with the flow's variables.

- `stop_at = "block"` or `"block:step"` stops the flow before that step. Steps count from 1.
- `break_on_failure = true` opens the session at a failed step, so that you can look at the screen. Without it, a failed flow ends as `golem run` ends it, and no session opens.
- `run = false` installs the app and does the setup, but runs no steps. The cursor is before the first step of the start block. Use it to edit a flow. Do not give `stop_at` with `run = false`.
- A relative `flow` path is in the project directory.

**Replay fake data.** `session_open` reports the seed of the session's fake data: `session open · … · seed 4711`. A failed `golem run` reports its seed too. `session_open(seed = 4711)` replays that data, as `golem run --seed 4711` does, with or without `flow`. A `[flow] seed` in a flow file has no effect.

### Long operations

A session runs one operation at a time.

- If an operation takes longer than the soft timeout, the call answers `pending`, with the operation and its phase:

  ```
  pending · op 1 session_open · phase: opening from e2e/login.test.toml · 45s
  → call wait() for the result
  ```

- `wait(timeout_s?)` waits for the running operation and returns its result. It can answer `pending` again.
- Sessions hold at most 3 devices at once, across all your MCP clients and shells. An open past the cap answers `pending` with the phase `waiting for a device`, and goes on when another session closes. Set `GOLEM_SESSION_MAX_DEVICES` in the server's `env` to change the cap. golem reads it when its background process starts, so restart the server after a change.
- A call made while an operation runs answers `busy`. `app_logs` is the exception: it does not wait, so it can show why a step hangs.
- `status` shows the running operation, or the last result.
- `cancel` stops the running operation. The teardown does not run.

## The draft and step status

Each step that passes in `act` goes into the session's flow draft, at the cursor. A session from a flow puts the cursor where the flow stopped. A new session starts a block named `main`.

`draft_show` returns the whole draft as `.test.toml` text. `draft_steps` lists the steps near the cursor. Each line has the step's address (`block:step`, steps count from 1), its status, the step and its comment:

```text
draft · 4 steps: 2 ✓ passed, 1 ? unverified, 1 ~ stale · cursor before login:3
[login] 3 steps · goto retry if { if_visible = "Error" }
  login:1 ✓ { action = "launch", app = "app", restart = true }  # Start clean
  login:2 ✓ { action = "tap", on_text = "Sign in" }
  ▸ cursor
  login:3 ? { action = "assert_visible", on_text = "Welcome" }
[retry] 1 step · next login
  retry:1 ~ { action = "tap", on_text = "Retry" }
```

| Status | Meaning |
|--------|---------|
| `✓` | The step passed in this session. |
| `·` | The step comes from the flow file and did not run in this session. |
| `?` | Unverified: the step did not run in its current form, or it is the first step after a change. |
| `~` | Stale: a step before it changed after it passed or was saved. |

A change, such as a new step from `act` or `record_only`, sets these statuses:

- The next active step becomes `?`, because the screen before it is different now. A `screenshot` is not an active step, so the step after it becomes `?`.
- Each later step that can run after the change becomes `~`. That is the rest of the block, then each block that a `branch`, `next` or the next block in the file leads to.

Only `?` goes into the file, as a `# unverified` line above the step. A step that passes in the session loses the line. `export_flow` gives the count of each status and lists the unverified steps.

`draft_steps` takes these arguments:

- `around`: `cursor` (the default), or a step address.
- `context`: the number of steps on each side. The default is 5.
- `block`: list the steps of one block instead.
- `limit`: show at most this many steps.

Each block's header line is always in the list, with its `next` and its branches.

### Edit the draft

These tools change a step that is already in the draft. They do not run the step. Each tool takes a step address, and each returns the steps around the change, because the addresses after the change can shift.

| Tool | What it does |
|------|--------------|
| `step_edit(at, step?, comment?)` | Replaces the step, its comment, or both. `comment = ""` removes the comment. |
| `step_delete(at)` | Removes the step and its comment. |
| `step_move(from, to)` | Moves the step so that its address becomes `to`. To add the step at the end of a block, give the address one past the block's last step. |

The status rules are the same as for a new step:

- `step_edit` keeps the status when only the comment changes, or when only the timeout gets larger. A step that passed with less time also passes with more. Any other change makes the step `?`, the next active step `?`, and the later steps `~`.
- `step_delete` makes the next active step `?`, and the later steps `~`.
- `step_move` counts as a delete where the step was, and the step is `?` where it lands. The step keeps its comments.

The cursor does not move. `act` still records at the cursor, which is where the device is in the flow.

### Blocks

A flow is a list of blocks. After a block's last step, the flow goes to the first `branch` whose condition is true. A block with branches ignores `next`: when no branch matches, the flow goes to the next block in the file. A block without branches goes to `next`, else to the next block in the file.

| Tool | What it does |
|------|--------------|
| `block_begin(name, next?)` | Moves the cursor to the end of block `name`. If the draft has no block with that name, the tool creates it at the end of the draft. |
| `block_link(block, next?, branches?)` | Sets the block's `next`, and adds branches. A branch has one condition (`if_visible`, `if_not_visible`, or `if_var` with `equals`, `matches` or `gte`) and a `goto`. |
| `block_rename(name, to)` | Renames the block, and each `next`, `goto` and `[flow] start` that names it. |
| `block_delete(name)` | Removes the block and its steps. The tool refuses while a `next`, a `goto` or `[flow] start` names the block, and it lists those names. |

A step that `act` records in a new block passed on the device. But the session does not run the other ways through a branch. Use `record_only` for the steps of a way that the session does not take.

### Run the draft again

After edits, some steps are `?` or `~`. `draft_run` runs the draft on the device to verify them. It runs the draft as it is in the session, not the file on disk. It does no setup and no teardown.

- `draft_run(restart = true)` runs from the first step of the start block.
- `draft_run()` resumes from the cursor.
- `stop_at = "block"` or `"block:step"` stops before that step. Without it, the run goes to the end of the flow.

Each step that passes becomes `✓` and loses its `# unverified` line. A step that fails stops the run, and the session stays open. The cursor goes where the run stops, so the next `act` records there. The result gives the count of steps that passed, where the run stopped, and the steps around the cursor.

`restart` does not clear the app's data. A flow clears state in its own steps, for example with `launch` and `restart = true`.

### Other draft tools

These tools change only the draft. They never touch the device, and they set no step status.

| Tool | What it does |
|------|--------------|
| `flow_set(name?, tags?, vars?, explicit_only?, start?)` | Sets fields of `[flow]`. `vars` merges into the flow's variables. |
| `apps_set(app)` | Adds a `[[flow.apps]]` entry, or replaces the fields of the entry with the same name. The entry can differ from the session's device. |
| `teardown_add(step, comment?)` | Adds a step to the `[[teardown]]`. The step does not run now. |
| `data_add(row)` | Adds a `[[data]]` row. A block with `for_each = "data"` runs once for each row, and its steps read the fields as `${_each.field}`. |
| `comment_add(text)` | Adds a comment line at the cursor. |
| `mixins_list` | Lists the project's mixins and the variables each uses. To use a mixin, run `act` with `{ action = "load_mixin", mixin = "name", vars = { … } }`. The draft records the `load_mixin` step, not the mixin's steps. |

## Example: write a new flow

The LLM writes a login flow for the app `app` in `golem.toml`:

```text
session_open(os = "android", app = "app")
act(step = '{ action = "launch", app = "app", restart = true }', comment = "Start from a clean launch")
tree()
probe(selector = '{ on_text = "Email" }')
act(step = '{ action = "type", on_text = "Email", input = "ada@example.test" }')
act(step = '{ action = "type", on_text = "Password", input = "${password}" }')
act(step = '{ action = "tap", on_text = "Sign in" }', comment = "Sign in with the test account")
act(step = '{ action = "assert_visible", on_text = "Welcome" }')
flow_set(name = "Login", tags = ["smoke"])
draft_show()
export_flow(path = "flows/login.test.toml")
session_close()
```

Then the LLM runs `golem run flows/login.test.toml` in a shell to prove that the flow passes.

- A step that fails does not go into the draft. Its result has the failure code, such as `EF404`, and a `fix:` line with the usual fix. The LLM fixes the selector with `probe` and runs `act` again.
- If a step passes but takes half its timeout or more, `act` adds a warning with a suggested timeout: `warning: took 4.1s of its 5s timeout · consider timeout = 9000`. The step is in the draft. On a slower device or a busy host, the same step can time out in `golem run`. To raise the timeout, use `step_edit`. A larger timeout keeps the step's status.
- `record_only` adds a step that does not run, for a path that the session does not take. The step gets an `# unverified` marker, and `export_flow` lists it.
- To add steps to a flow that exists, open the session from that flow with `stop_at`. The new steps go in where the flow stopped. The export keeps the file's comments, key order and whitespace.

## Example: edit a flow that exists

The app renamed its "Sign in" button to "Log in", so step 3 of `flows/login.test.toml` no longer matches. The LLM fixes it, proves the flow again, and saves the file:

```text
session_open(os = "android", flow = "flows/login.test.toml", run = false)
draft_steps(block = "login")
draft_run(stop_at = "login:3")
probe(selector = '{ on_text = "Log in" }')
step_edit(at = "login:3", step = '{ action = "tap", on_text = "Log in" }')
draft_run(restart = true)
draft_steps()
export_flow(path = "flows/login.test.toml")
session_close()
```

- `run = false` opens the session before the first step, without running the flow.
- The first `draft_run` brings the app to the screen of the broken step, so that `probe` reads the real screen.
- After `step_edit`, step 3 and the step after it are `?`. The `draft_run` from the start makes every step that passes `✓`, and removes the `# unverified` lines.
- `export_flow` writes over the file without `overwrite`, because the session opened from it.

## Example: debug an app issue

The user reports that the app crashes after Save on the profile screen:

```text
session_open(os = "ios", app = "app", flow = "flows/profile.test.toml", stop_at = "edit")
screenshot()
act(step = '{ action = "type", on_text = "Name", input = "Ada" }')
act(step = '{ action = "tap", on_text = "Save" }', tree = true)
app_logs(since = 60)
session_close(teardown = false)
```

`app_logs` shows the crash lines first: the signal or the exception, and the top frames. The LLM reports the crash and the steps that cause it. No export is necessary.

`app_logs` reads `adb logcat` on Android and the unified log on an iOS simulator. A physical iOS device is not supported. On iOS, `print` output does not reach the log: use `NSLog`, `os_log` or `Logger`.

## Upgrades and troubleshooting

- **After an upgrade,** restart the golem MCP server in your client. Until you do, the client uses the old golem, and its tools can fail with an "is older than the running daemon" error.
- **An open session delays `golem run` after an upgrade.** The new `golem run` waits for the sessions of the old golem to end, up to `GOLEM_DAEMON_WAIT` seconds (default 300). Close the session, or stop the MCP server, to continue.
- **Two golem versions,** such as an npm project version and a global brew version, interfere with each other. Use the same version for both, or set `GOLEM_SOCKET` to a different path for each one.
- **`adb` or `xcrun` not found:** the client did not give golem your shell `PATH`. Set `PATH` and `ANDROID_HOME` in the server's `env`, as in the [Claude Desktop](#claude-desktop) example.
- **The log** of the golem background process is `~/.golem/golem.log`. See [The daemon](cli-reference.md#the-daemon).

---

For golem developers: [MCP context](mcp-context.md) shows, on one page, everything an MCP client receives from `golem mcp`.
