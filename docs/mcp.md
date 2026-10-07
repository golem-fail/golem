# MCP Server

*Another mind speaks the words; the golem does the work.*

← [Back to README](../README.md) · See also [CLI Reference](cli-reference.md#golem-mcp) · [Actions Reference](actions-reference.md)

`golem mcp` is an [MCP](https://modelcontextprotocol.io) server. An LLM client, such as Claude Code, Codex, OpenCode, Gemini CLI or Claude Desktop, uses it to drive an iOS or Android device one step at a time. Use it for two tasks:

- **Write a flow.** The LLM runs steps on a live device. Each step that passes goes into a flow draft. The LLM then exports the draft as a `.test.toml` file that `golem run` runs.
- **Debug an app.** The LLM taps through the app, reads the screen, takes screenshots and reads the app's device log, then reports what it found.

The tool list and each tool's arguments are in the [CLI reference](cli-reference.md#golem-mcp). This page tells you how to set up a client and how a session works.

## Contents

- [Setup](#setup)
  - [Claude Code](#claude-code)
  - [Codex CLI](#codex-cli)
  - [OpenCode](#opencode)
  - [Gemini CLI](#gemini-cli)
  - [Claude Desktop and other GUI clients](#claude-desktop-and-other-gui-clients)
  - [Install channels](#install-channels)
  - [Timeouts](#timeouts)
- [The step notation](#the-step-notation)
- [The visible tree decides](#the-visible-tree-decides)
- [Sessions](#sessions)
  - [A session from a flow](#a-session-from-a-flow)
  - [Long operations](#long-operations)
- [Example: write a new flow](#example-write-a-new-flow)
- [Example: debug an app issue](#example-debug-an-app-issue)
- [Upgrades and troubleshooting](#upgrades-and-troubleshooting)

## Setup

Before you start, run `golem doctor`.

`golem mcp --print-config <client>` prints a config block for `claude`, `codex`, `opencode`, `gemini` or `desktop`. The block holds the absolute path of the `golem` that you ran.

**Boot a device first.** `session_open` uses a simulator, an emulator or a connected device that is already booted. It does not boot a device, as `golem run` does. `devices` lists each device and its state.

### Claude Code

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

Check the server with `/mcp` in a Claude Code session, or with `claude mcp list`.

### Codex CLI

```bash
codex mcp add golem -- golem mcp --project /path/to/your/project
```

The command writes this block to `~/.codex/config.toml`:

```toml
[mcp_servers.golem]
command = "golem"
args = ["mcp", "--project", "/path/to/your/project"]
```

Codex can start the server from any directory, so give `--project`. A trusted project can also hold the block in `.codex/config.toml`. Check the server with `codex mcp list`.

### OpenCode

OpenCode has no command that adds a server. Add the server to `opencode.json` at the project root, or to `~/.config/opencode/opencode.json` for all your projects:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "golem": { "type": "local", "command": ["golem", "mcp"], "enabled": true }
  }
}
```

In the global file, add `"--project", "/path/to/your/project"` to `command`. Check the server with `opencode mcp list`.

### Gemini CLI

```bash
gemini mcp add golem golem mcp
```

- The default scope is `project`: the command writes `.gemini/settings.json` in the project. Commit that file to share the server with your team.
- `--scope user` writes `~/.gemini/settings.json`. Then add `--project /path/to/your/project` to the server's arguments.

The block has the same form as the Claude Code block:

```json
{ "mcpServers": { "golem": { "command": "golem", "args": ["mcp"] } } }
```

Check the server with `/mcp` in a Gemini CLI session, or with `gemini mcp list`. `gemini mcp list` shows the server as connected only in a trusted folder.

### Claude Desktop and other GUI clients

Add the same `mcpServers` block to the client's config file. For Claude Desktop on macOS, the file is `~/Library/Application Support/Claude/claude_desktop_config.json`. Then restart the client.

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

- Use the absolute path of `golem`. A GUI app does not get your shell `PATH`.
- Set `PATH` and `ANDROID_HOME` in `env`. Without them, golem cannot find `adb` or `xcrun`.
- `golem mcp --print-config desktop` writes this block with your current `PATH` and `ANDROID_HOME`.

### Install channels

| Install | Command |
|---------|---------|
| brew or the install script | `golem mcp` |
| npm dev dependency | `npx golem mcp`, or the absolute path `node_modules/.bin/golem` with the argument `mcp` |

### Timeouts

A client stops waiting for one tool call after a limit. golem answers every call before the **soft timeout** (default 45 s). If the work is not done by then, the call answers `pending`, and the LLM calls `wait` for the result. See [Long operations](#long-operations).

- **Codex:** `tool_timeout_sec` defaults to 60. Keep it above the soft timeout. `startup_timeout_sec` defaults to 10. The server starts in well under that time.
- **Claude Code:** `MCP_TOOL_TIMEOUT` (milliseconds) sets the limit for every server, and `"timeout"` on one server in `.mcp.json` sets it for that server. The defaults are much longer than the soft timeout.
- **Gemini CLI:** `"timeout"` on the server (milliseconds) defaults to 600000, which is longer than the soft timeout.
- **OpenCode:** `"timeout"` on the server (milliseconds, default 5000) applies only while the server starts. golem starts in well under that time.
- **Other clients:** if a client's limit is under 45 s, start the server with `--soft-timeout` below that limit.

## The step notation

A step is one TOML inline table on one line. It is the same text as a step in a flow file:

```toml
{ action = "tap", on_text = "Sign in" }
{ action = "type", on_accessibility_label = "Email", input = "ada@example.test" }
{ action = "assert_visible", on_text = "Welcome" }
```

`act` runs one step. `probe` takes the same table without `action`, and shows what the selector matches. `actions_help` lists every action, and `actions_help(action)` shows the fields and examples of one action.

## The visible tree decides

golem tests as a person does. A step finds its element only in the visible tree: the elements on the screen, not the elements that are off the screen or hidden under other elements. `tree` returns the visible tree. `tree(full = true)` returns every element, marked as a hint. Use the full tree to find where to scroll, but never to decide that a step passed. `probe` lists off-screen matches as hints for the same reason.

## Sessions

A session holds one device, one app, the variables and a flow draft. Each MCP server has one session at a time.

1. `session_open` picks the device and the app. It can first run a flow (see below).
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
- A relative `flow` path is in the project directory.

### Long operations

A session runs one operation at a time.

- If an operation takes longer than the soft timeout, the call answers `pending`, with the operation and its phase:

  ```
  pending · op 1 session_open · phase: opening from e2e/login.test.toml · 45s
  → call wait() for the result
  ```

- `wait(timeout_s?)` waits for the running operation and returns its result. It can answer `pending` again.
- A call made while an operation runs answers `busy`. `app_logs` is the exception: it does not wait, so it can show why a step hangs.
- `status` shows the running operation, or the last result.
- `cancel` stops the running operation. The teardown does not run.

## Example: write a new flow

The LLM writes a login flow for the app `app` in `golem.toml`:

```text
session_open(platform = "android", app = "app")
act(step = '{ action = "launch", app = "app", restart = true }', comment = "Start from a clean launch")
tree()
probe(selector = '{ on_accessibility_label = "Email" }')
act(step = '{ action = "type", on_accessibility_label = "Email", input = "ada@example.test" }')
act(step = '{ action = "type", on_accessibility_label = "Password", input = "${password}" }')
act(step = '{ action = "tap", on_text = "Sign in" }', comment = "Sign in with the test account")
act(step = '{ action = "assert_visible", on_text = "Welcome" }')
flow_set(name = "Login", tags = ["smoke"])
draft_show()
export_flow(path = "flows/login.test.toml")
session_close()
```

Then the LLM runs `golem run flows/login.test.toml` in a shell to prove that the flow passes.

- A step that fails does not go into the draft. The LLM fixes the selector with `probe` and runs `act` again.
- `record_only` adds a step that does not run, for a path that the session does not take. The step gets an `# unverified` marker, and `export_flow` lists it.
- To add steps to a flow that exists, open the session from that flow with `stop_at`. The new steps go in where the flow stopped. The export keeps the file's comments, key order and whitespace.

## Example: debug an app issue

The user reports that the app crashes after Save on the profile screen:

```text
session_open(platform = "ios", app = "app", flow = "flows/profile.test.toml", stop_at = "edit")
screenshot()
act(step = '{ action = "type", on_accessibility_label = "Name", input = "Ada" }')
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
- **`adb` or `xcrun` not found:** the client did not give golem your shell `PATH`. Set `PATH` and `ANDROID_HOME` in the server's `env`, as in the [Claude Desktop](#claude-desktop-and-other-gui-clients) example.
- **The log** of the golem background process is `~/.golem/golem.log`. See [The daemon](cli-reference.md#the-daemon).
