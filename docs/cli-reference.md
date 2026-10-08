# CLI Reference

*The words you speak to it.*

← [Back to README](../README.md) · See also [App Install](app-install.md) · [Test Structure](test-structure.md)

## Contents

- [`golem run`](#golem-run)
- [`golem tree`](#golem-tree)
- [`golem probe`](#golem-probe)
- [`golem mcp`](#golem-mcp)
- [`golem session`](#golem-session)
- [`golem devices`](#golem-devices)
- [`golem init`](#golem-init)
- [`golem create <name>`](#golem-create-name)
- [`golem install-script`](#golem-install-script)
- [`golem a11y-extract <png>`](#golem-a11y-extract-png)
- [`golem doctor`](#golem-doctor)
- [The daemon](#the-daemon)

## `golem run`

Run one or more test flows.

```bash
golem run [FILES...] [OPTIONS]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `FILES...` | Flow files or directories. If empty, auto-discovers from current directory. |

**Options:**

| Flag | Description |
|------|-------------|
| `--platform <ios\|android>` | Force a single platform (overrides flow device config) |
| `--tag <TAG>` | Filter flows by tag. Repeatable. Use `\|` within a value for OR. |
| `--var <KEY=VALUE>` | Set a variable (highest priority, overrides flow vars). Repeatable. |
| `--output <FORMAT>` | Stdout format: `human` (default), `json`, `junit`, `toon`. Repeatable. |
| `--output-dir <PATH>` | Results directory (default: `.golem/results`). JSON + toon always written. |
| `--no-results` | Disable all file output (screenshots, recordings, reports) |
| `--seed <N>` | Deterministic seed for fake data generation. Seed shown in all output formats for reproducibility. |
| `--start <BLOCK>` | Start execution at a named block (skips app lifecycle, assumes app in correct state) |
| `--max-concurrency <N>` | Cap on FlowRuns running at once. Caps only — the host-headroom guard (RAM, device availability) still applies, so effective parallelism is the lower of the two |
| `--max-device-wait <DUR>` | Hard cap on how long a flow waits in the device queue before failing with "no device available" (`30m`, `1h`, `90s`, `1h30m`). Default: unbounded. Beats `[options].max_device_wait` in `golem.toml` and a flow's own `[flow.options]`. |
| `--record` | Enable auto screen recording for every block. Loses to `--no-record`. |
| `--no-record` | Force-disable recording everywhere — beats `--record`, flow options, and per-block opts. |
| `--trace` | Forensic capture: forces recording on (beats `--no-record`) + writes screenshot + accessibility-tree at every step boundary to `results/.../trace/`. ~200ms/step overhead — investigation only. |
| `--repeat <N>` | Repeat the whole suite N times (1..=100). Each run writes to `{output-dir}/run_{i}/`. The orchestrator fans every FlowRun out N times, so identical-device pools parallelise for free. A flake summary is printed at the end. |
| `--no-clean` | Skip app data clear between flows (not yet implemented) |
| `--no-teardown` | Skip teardown blocks |
| `--keep-devices` | Keep devices running after completion (not yet wired) |
| `--no-perf` | Disable performance capture |
| `--a11y <off\|critical\|relaxed\|strict>` | Override every flow's accessibility audit level (default `relaxed`). `off` disables; `critical` runs tree checks only; `relaxed` adds opportunistic contrast; `strict` forces a per-block screenshot + AAA bands |
| `--a11y-min-confidence <0.0–1.0>` | Override every flow's `a11y_min_confidence`: drop a11y findings below this confidence. `0` surfaces every heuristic finding, higher keeps only confident ones. Wins over `[flow.options]` and the level default. |
| `--rebuild` | Bypass the persistent install cache for this run (rebuild + reinstall every app on every device). Cache is still written after a successful build, so the next run benefits. |
| `--no-build` | Skip build+install entirely. If the device already has the bundle, golem trusts it and runs flows; if not, the flow fails loudly. The cache is left untouched. Use when iterating on flow files against a known-good binary. |
| `--dev` | Iterate against a dev server you run yourself (Expo/Metro) instead of rebuilding per change. Implies `--no-build`. golem waits for the dev server, then each flow's relaunch re-fetches the current bundle — so a JS edit needs no rebuild or reinstall. golem never starts the bundler. |
| `--dev-port <n>` | Port the `--dev` dev server listens on (default `8081`, Metro's) |
| `--dev-wait <dur>` | How long `--dev` waits for the dev server before failing with `H503` (default `30s`; e.g. `2m`) |
| | `--dev` also checks the app's bundle builds before running, and reports a step that fails against a React Native error overlay as `A501` rather than a missing selector |
| `--verbose` | Show substeps (scroll coordinates, strategies, tree stats) + plan summary (flow runs, install matrix, device availability) + cache hits/misses |
| `--debug` | Show driver diagnostics (WebKit/CDP) and per-line install-script stderr |

**Examples:**

```bash
# Run on Android only
golem run flows/ --platform android

# Run with variables
golem run flows/login.test.toml --var EMAIL=test@example.com --var PASSWORD=secret

# Multiple output targets
golem run flows/ --output json --output junit   # json+junit to stdout, all results to .golem/results/

# Filter by tag
golem run flows/ --tag smoke
golem run flows/ --tag "auth|login"

# Verbose mode for debugging scroll behavior
golem run flows/scroll.test.toml --verbose
```

## `golem tree`

Inspect the live UI element hierarchy of one device.

```bash
golem tree [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--os <OS>` | Consider only devices with this OS, in the flow syntax: `ios`, `android`, `ios:26` (any 26.x), `ios:26+` or `ios:latest` |
| `--device <ID\|NAME>` | The device: a UDID or serial, a name, or part of either (case-insensitive) |
| `--bundle <ID>` | The bundle ID of the app to read |
| `--app <NAME>` | The app to read, by its name in the `golem.toml` `[[apps]]` registry |
| `--full` | Show the full tree, not only what is on screen. The output says it is a hint only: a step targets and asserts against the visible tree |
| `--output <toon\|json>` | `toon` (default): one indexed line per selectable element, see [TOON tree](output-formats.md#toon-tree). `json`: the element tree |
| `--json` | Same as `--output json` |
| `--verbose` | Show metadata (CDP status, enrichment, keyboard, safe area) and the debug element tree |

**Device.** golem reads a device that runs: a booted simulator or emulator, or a connected physical device. It does not boot one; `golem session start` does.

- If only one running device fits `--os`, golem uses it.
- With `--device`, an exact UDID or serial wins, then an exact name, then part of either.
- If more than one running device fits, or none does, the command fails. It lists the running devices that fit, in the format of [`golem devices`](#golem-devices), so that you can retry with `--device`:

```text
Error: 2 running ios devices; pick one with --device <id or name>:

iOS Simulators:
  iPhone 16  ios:18.6  phone  booted  A66D93B7-B5C0-426E-87E2-276098961C99
  iPhone 17  ios:26.5  phone  booted  B9100F0F-54EB-4DE3-9DFD-CB5FBC8FAB2B

All devices, shut-down ones too: golem devices
```

golem reuses the device's running companion, or starts one.

**App.** `--bundle` is used as given. `--app` looks the bundle up in `golem.toml`. With neither, golem reads the registry's only app. If the registry has several apps or none, no bundle is set: on iOS the companion reads the app it last launched, and on Android the tree covers the whole screen.

## `golem probe`

Show what a selector matches on one device, without acting.

```bash
golem probe '{ on_text = "Sign in" }' [OPTIONS]
```

The selector is one TOML inline table, in the same notation as a step in a flow file's `steps = [ … ]` array. The outer braces are optional. You can paste the step you plan to run: `probe` ignores its `action`. `probe` never fails: it always exits 0.

| Flag | Description |
|------|-------------|
| `--timeout <MS>` | Poll for up to this long while nothing visible matches (default 0: check the screen once) |
| `--os <OS>` | Consider only devices with this OS, in the flow syntax: `ios`, `android`, `ios:26` (any 26.x), `ios:26+` or `ios:latest` |
| `--device <ID\|NAME>` | The device, chosen as `golem tree` chooses it |
| `--bundle <ID>` | The bundle ID of the app |
| `--app <NAME>` | The app, by its name in the `golem.toml` `[[apps]]` registry |
| `--output <toon\|json>` | `toon` (default) or `json` |

The output shows:

- the visible matches, with their `[n]` from the [TOON tree](output-formats.md#toon-tree), in the order a step picks from
- the match a step would act on (the first), and a warning when more than one element matches
- the element that each relational anchor (`below`, `contains` and the others) resolved to
- on a miss, the matches left after each clause, so the output shows which clause removed them
- matches in the full tree that are off-screen, as a hint only, with the direction `auto_scroll` would scroll

`probe` and a step use the same visible-tree matching, so `probe` reports the element that the step acts on. `probe` only reads the screen, so it also works on a device that a run is using.

```text
$ golem probe '{ on_text = "Action" }' --app app
android/Pixel 8 Pro API 36 probe { on_text = "Action" } · 2 visible matches · act picks [15]
 [15] Button "Action" 714,1098 225x132 @826,1164  ·button·has_text·short_text·
 [16] Button "Action" 969,1098 225x132 @1081,1164  ·button·has_text·short_text·
warn: more than one element matches, and act picks the first. Add index, or a tighter selector
```

## `golem mcp`

Run an MCP server over stdio, so an LLM client (Claude Code, Codex, Cursor, Claude Desktop and others) can drive a device one step at a time.

```bash
golem mcp [--project <DIR>] [--soft-timeout <SECS>]
golem mcp --print-config <CLIENT>
```

Setup for each client, the session rules and two example sequences are in [golem as an MCP server](mcp.md).

| Flag | Description |
|------|-------------|
| `--project <DIR>` | The project directory that holds `golem.toml`. Without it, golem searches up from the working directory. `session_open` can also name a project. |
| `--soft-timeout <SECS>` | How long a tool waits for its operation before it answers `pending`. Without it, golem picks two thirds of the connecting client's limit for one call, at most 120 s, or 45 s for a client it does not know. See [Timeouts](mcp.md#timeouts). |
| `--print-config <CLIENT>` | Print the config block that adds this server to a client, with the absolute path of this `golem`, then exit. `CLIENT` is `claude`, `codex`, `opencode`, `gemini`, `copilot`, `goose`, `zed`, `continue` or `desktop`. The `desktop` and `zed` blocks also set `env` to your `PATH` and `ANDROID_HOME`. [MCP Server](mcp.md#setup) says which file each block goes in, and which block the other clients use. |

The server starts without device work.

**The device.** `session_open` picks its device as a flow's `[[flow.apps.devices]]` would:

- `os` takes the flow syntax: `ios`, `android`, `ios:26` (any 26.x), `ios:26+`, or `ios:latest` (the newest OS on the host, in any state). `type` is `phone` or `tablet`. `device` names one device by UDID, serial, name, or part of either, in any state.
- A running device that fits and is free wins: a simulator or emulator before a physical device, then the newest OS. A device that a run or another session holds is skipped.
- If no running device fits, golem boots the fitting shut-down device with the newest OS, as `golem run` does. The open answers `pending` with the phase `booting …` while it boots. `boot = false` refuses instead.
- Sessions hold at most 3 devices at once, across every MCP server and shell on the host (`GOLEM_SESSION_MAX_DEVICES`). An open past the cap waits with the phase `waiting for a device: 3 of 3 held by sessions (…)`, and goes on when a session closes. `cancel` ends the wait. Runs from `golem run` do not count toward the cap.
- When the daemon exits, it shuts down the devices that golem booted, unless a `golem run --keep-devices` used it.
- With `flow`, the flow's device constraint fills each of `os`, `type` and `device` that the call leaves out. A flow that runs on both platforms needs `os`.
- `session_open` refuses an argument it does not know, so a misspelt name fails instead of being ignored. The session runs in the daemon and belongs to this server: when the client stops the server, the session ends and its device is released. Stdout carries JSON-RPC only.

| Tool | Description |
|------|-------------|
| `devices(os?)` | Every device in any state, with the port of a live companion. `os` filters as in `session_open` |
| `session_open(os?, type?, device?, boot?, bundle?, app?, project?, idle_timeout_s?, flow?, stop_at?, break_on_failure?, teardown?, vars?)` | Open a session on one device and app; see "The device" below. It ends after `idle_timeout_s` (default 1800) with no operation. With `flow`, golem first runs that flow as `golem run` would (install, apps, launch, steps) and opens the session where it stops; see below |
| `session_close(teardown?)` | Close the session and release the device. For a session opened from a flow, the flow's `[[teardown]]` runs unless `teardown = false` |
| `act(step, comment?, tree?, format?)` | Run one step, with the same element resolution, auto-scroll, settle and timeout as a step in a flow. `tree = true` adds the visible tree after the step |
| `probe(selector, timeout_ms?, format?)` | As `golem probe` |
| `tree(full?, format?)` | The [TOON tree](output-formats.md#toon-tree); `full = true` is a hint only |
| `screenshot` | The screen as a PNG image |
| `wait(timeout_s?)`, `status`, `cancel` | The running operation: its result, its state, or stop it |
| `app_logs(since?, filter?, limit?, app?)` | The app's device log, see below. It does not wait for a running operation |
| `draft_show` | The flow draft: the steps that passed in `act`, as `.test.toml` text |
| `draft_steps(around?, context?, block?, limit?)` | The draft's steps near the cursor, or one block's steps, each with its `block:step` address and status |
| `export_flow(path, overwrite?)` | Check the draft as `golem run` would, then write it, with the count of each status and the unverified steps |
| `flow_set(name?, tags?, vars?, seed?, explicit_only?, start?)` | Set `[flow]` fields of the draft |
| `apps_set(app)` | Add or replace a `[[flow.apps]]` entry: `bundle`, `devices`, `permissions`, `install_script` |
| `block_begin(name, next?)` | Record the next steps into a block, creating it if needed |
| `block_link(block, next?, branches?)` | Set a block's `next`, and add branches (`if_visible`, `if_not_visible` or `if_var`, then `goto`) |
| `teardown_add(step, comment?)` | Add a step to the draft's `[[teardown]]`; it does not run |
| `data_add(row)` | Add a `[[data]]` row |
| `comment_add(text)` | Add a comment line where the next step goes |
| `record_only(step, comment?)` | Record a step without running it, marked `# unverified`, for a path the session does not take |
| `mixins_list` | The project's mixins and the vars each expects; run one with `act` and `action = "load_mixin"` |
| `actions_help(action?)` | The step notation and every action, or one action's reference |

**A session from a flow.** `session_open(flow = "e2e/checkout.test.toml")` runs the flow on the chosen device, then keeps the device, the driver and the flow's variables for the session:

- `stop_at = "block"` or `"block:step"` stops before that step (steps count from 1). Without it, the session opens where the flow ends.
- `break_on_failure = true` opens the session at a failed step. Without it, a failed flow ends as `golem run` would, teardown included, and no session opens.
- The flow's `[[teardown]]` runs only on `session_close`. A dropped connection or the idle timeout skips it, and `teardown = false` skips it on every end.
- The flow must run once on one device: a flow that expands to several runs or devices is refused, and so is a `stop_at` in a `for_each` block.

**App logs.** `app_logs` reads the app's lines from the device log: `adb logcat` on Android, the unified log (`log show`) on an iOS simulator. A physical iOS device is not supported.

- `since` is seconds back from now. Without it, the lines start when the session opened.
- `filter` keeps the lines whose tag or message contains the text, in any case.
- Crash lines come first: a fatal error, an uncaught exception, an ANR, a signal that killed the app. Then come the other lines. Each part has its own `limit` (default 200), so an old crash still shows when many other lines follow it.
- Each crash shows its first 12 lines: the signal or exception, the abort message and the top frames. When the limit cuts the crash lines, the newest crashes stay.
- On Android, golem keeps the lines that the app's uid logged, so the lines before a restart stay. Lines from other processes are kept when they name the app's package, such as the line where ActivityManager restarts it. A process that runs under another uid, such as the sandboxed WebView renderer, is not shown unless its line names the package.
- On iOS, the lines come from the app's process. The app's `print` output does not reach the unified log; `NSLog`, `os_log` and `Logger` do. A process that a signal kills logs nothing itself, so golem adds SpringBoard's `Process exited` line for the app, which names the signal.

**The flow draft.** Each step that passes in `act` goes into the session's draft, as written (a `${var}` stays a reference), with `comment` on its own line above it. A step that fails does not. A session opened from a flow drafts that file: steps go in where the flow stopped (before `stop_at`, or before the failed step), else at the end of the last block, and the file's comments, key order and whitespace stay as they were. A step takes the form its block already uses, `steps = [ … ]` or `[[block.steps]]`; a one-line `steps = [{ … }]` becomes one step per line. A new session drafts a new flow with its app and a `main` block. `export_flow` refuses a draft that does not validate, and refuses to replace a file the session did not open from unless `overwrite = true`.

**Step status.** Each step of the draft has a status, which `draft_steps` and `export_flow` show: `✓` passed in this session, `·` comes from the file and did not run in this session, `?` unverified, `~` stale. A change to the draft (a step from `act`, `record_only`) makes the next active step `?`, because the screen before it changed. Each later step that can run after the change becomes `~`: the rest of the block, then each block that a `branch` target, `next` or the next block in the file leads to. A `screenshot` is not an active step. Only `?` is in the file, as a `# unverified` line above the step; a step that passes in the session loses it.

A session runs one operation at a time. A call made while another runs answers `busy`. A call that takes longer than the soft timeout answers `pending`, and `wait` returns its result. `format = "json"` returns JSON instead of TOON.

## `golem session`

Keep a device session open between shell commands. A session keeps the device, the variables and a flow draft until it stops. The daemon holds the session, so each command is a new process.

```bash
golem session start [--name <NAME>] [--os …] [--type …] [--device …] [--no-boot] [--app …] [--flow <FILE> …]
golem session do '{ action = "tap", on_text = "Sign in" }' [--comment <TEXT>] [--tree]
golem session probe '{ on_text = "Sign in" }' [--timeout <MS>]
golem session tree [--full]
golem session screenshot <PATH>
golem session logs [--since <SECS>] [--filter <TEXT>] [--limit <N>] [--app <APP>]
golem session export <PATH> [--overwrite]
golem session stop [--no-teardown]
golem session list
```

| Command | Description |
|---------|-------------|
| `start` | Open a session. `--os`, `--type`, `--device`, `--no-boot`, `--bundle` and `--app` work as the `session_open` arguments of [`golem mcp`](#golem-mcp). `--idle-timeout <SECS>` defaults to 1800. With `--flow`, golem first runs that flow as `golem run` would and opens the session where it stops; `--stop-at`, `--break-on-failure`, `--no-teardown` and `--var KEY=VALUE` work as in the MCP `session_open` |
| `do` | Run one step, with the session's variables, as a step in a flow runs. A step that passes goes into the draft, with `--comment` above it. The exit code is 1 when the step fails |
| `probe`, `tree` | As `golem probe` and `golem tree`, on the session's device |
| `screenshot` | Write the screen to a PNG file |
| `logs` | The app's device log, as the MCP `app_logs` tool returns it |
| `export` | Check the draft as `golem run` would, then write it as a `.test.toml` |
| `stop` | Close the session and release the device. For a session started with `--flow`, the flow's `[[teardown]]` runs unless `--no-teardown` |
| `list` | Each open session: its name, device and state |

- **Names.** Every command takes `--name` (default `default`). Each name is one session, so two sessions can hold two devices.
- **Idle timeout.** A session that runs no command for `--idle-timeout` seconds stops, without the teardown. While a session is open, the daemon does not exit.
- **One command at a time.** A second command while one runs in the same session fails as busy. A command that runs longer than 10 seconds prints what it is doing on stderr while it waits.
- **Upgrades.** `start` replaces a daemon from another golem build, as `golem run` does: that daemon waits for its sessions to stop first. The other commands use the daemon that is running, whatever its build, because the session lives in it.

```text
$ golem session start --app app
session open · android/Pixel 8 Pro API 36 (emulator-5554) · app fail.golem.test
$ golem session do '{ action = "read", on_below = "Counter", save_to = "count" }'
+read:on_below="Counter" d:89 t:1/357
$ golem session do '{ action = "assert_visible", on_text = "${count}", on_below = "Counter" }'
+assert_visible:on_text="+" on_below="Counter" d:93 t:1/357
$ golem session stop
session default stopped
```

## `golem devices`

List all simulators, emulators and physical devices, in any state. Each row shows the name, the OS, the type, the state and the UDID (iOS) or serial (Android). Pass the UDID or serial to `--device` when two devices share a name.

```text
iOS Simulators:
  iPhone 16  ios:18.6  phone  booted    A66D93B7-B5C0-426E-87E2-276098961C99
  iPhone 16  ios:26.5  phone  shutdown  E5F6A7B8-0C1D-4E2F-9A3B-5C6D7E8F9A0B
```

## `golem init`

Scaffold a new project: creates `golem.toml`, `flows/`, `__fixtures__/`, `__mixins__/`, and `.golem/`.

## `golem create <name>`

Create a new flow template at `flows/<name>.test.toml`.

## `golem install-script`

Interactively scaffold an install script for an app in your project. Prompts for framework (native-ios, native-android, tauri), the relevant build config (xcode project/scheme, gradle root/module, tauri CLI runner), discovers candidates automatically where possible, and writes a bash script under `scripts/`. Optionally updates `golem.toml` with a matching `[[apps]]` entry so flows inherit the script by name.

See [App Install](app-install.md) for the full resolution and execution model.

## `golem a11y-extract <png>`

Read the audit embedded in an annotated a11y screenshot (`strict` runs write
`*_a11y.png` with the findings + context baked in as PNG metadata — see
[accessibility.md](accessibility.md#embedded-metadata)). Prints every finding in
human form (marker, severity, message, detail, confidence, pixel bounds) and the
`golem run …` command to **replay that exact run** — `--seed`, `--a11y`, and
`--platform` reconstructed from the metadata, with the flow file located by
matching its name against the project's `*.test.toml` files (run it from inside
the project).

| Flag | Description |
|------|-------------|
| `--json` | Print the raw embedded `Golem-Audit` JSON instead of the human report (for tooling). |

Errors (non-zero exit) if the PNG wasn't produced by golem — it requires the
`Software = Golem` metadata stamp and refuses to interpret a foreign image.

## `golem doctor`

Diagnose the environment. Two modes, combinable; where a tool exposes one, the
detected version is shown (`found 6.1.1`).

| Flag | Description |
|------|-------------|
| *(none)* | **Runtime** checks (default): what's needed to *drive* a device. |
| `--build` | **Build** checks: what's needed to *build* golem from source. |
| `--runtime` | Runtime checks explicitly; combine with `--build` to check everything. |

**Runtime** — the `golem` binary is self-contained (companions baked in), so this
checks only what a prebuilt binary *can't* carry, each with a copy-paste remedy:

- `~/.golem` writable (companions extract here)
- `adb` on PATH + the Android companion embedded
- `xcrun` / `simctl` on PATH + the iOS companion embedded (macOS only; *n/a* elsewhere)
- at least one emulator/simulator available to boot, or a connected device
  (informational — golem boots one on demand)
- `ffmpeg` (optional — lets the a11y audit and `--trace` reuse a frame from an
  existing recording instead of an extra live screenshot; recording works without it)

Exits non-zero when the host can drive **no** platform. A single missing CLI is a
warning, not a failure, as long as the other platform is drivable. golem also
prints the relevant runtime lines automatically when a run dead-ends on a missing
device.

**Build** (`--build`) — the contributor / release-box path:

- Rust toolchain (`cargo`)
- JDK + Android SDK (`ANDROID_HOME`) — to build the Android companion
- `xcodebuild` — to build the iOS companion (macOS only; *n/a* elsewhere)

Exits non-zero without Rust, or when no companion is buildable.

## The daemon

One background process, the daemon, owns the devices. Every `golem run` is a client: it hands its flows to the daemon and waits for them only. Two runs at the same time therefore share one device pool and never take the same device.

- **Start.** The first command that finds no daemon starts `golem daemon` as a detached process and connects to it. Concurrent commands start exactly one daemon.
- **Exit.** The daemon exits after 45 seconds with no client connected and no session open. Before it exits, it shuts down the simulators and emulators that golem booted. If any run during the daemon's life passed `--keep-devices`, it leaves them running.
- **Versions.** A command uses the daemon only if both are the same golem: the same version and the same build. A daemon left running by an older version, or by an earlier build of this version, finishes its runs and exits, and the command then starts its own; meanwhile the command prints a wait line. A command older than the running daemon fails at once and names both versions and binaries.
- **Cancel.** A run whose command ends early (Ctrl-C, killed) is cancelled in the daemon: its devices are released at once and the processes it started are stopped.
- **Environment.** Each run sends its environment variables and working directory. The processes golem starts for that run (install scripts, `bash`, `run`) get those, not the daemon's.
- **Log.** The daemon writes its own output to `golem.log` next to its socket (`~/.golem/golem.log`). Host diagnostics that are not run events, such as a failed WebView inspector setup, appear there and not in the run's output.

| Environment variable | Description |
|----------------------|-------------|
| `GOLEM_SOCKET` | The daemon's socket (default `~/.golem/golem.sock`). A different socket gives a separate daemon with its own devices; its lock and log sit next to it. |
| `GOLEM_DAEMON_IDLE_SECS` | Seconds with no client before the daemon exits (default 45). |
| `GOLEM_DAEMON_WAIT` | Seconds a command waits for an outdated daemon to finish its runs and exit (default 300). |
| `GOLEM_SESSION_MAX_DEVICES` | The most devices that sessions hold at once, across every client of the daemon (default 3). Read when the daemon starts. |
