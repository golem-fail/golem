# CLI Reference

*The words you speak to it.*

← [Back to README](../README.md)

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
| `--max-concurrency <N>` | Max parallel devices (not yet implemented) |
| `--record` | Enable auto screen recording for every block. Loses to `--no-record`. |
| `--no-record` | Force-disable recording everywhere — beats `--record`, flow options, and per-block opts. |
| `--trace` | Forensic capture: forces recording on (beats `--no-record`) + writes screenshot + accessibility-tree at every step boundary to `results/.../trace/`. ~200ms/step overhead — investigation only. |
| `--repeat <N>` | Repeat the whole suite N times (1..=100). Each run writes to `{output-dir}/run_{i}/`. The orchestrator fans every FlowRun out N times, so identical-device pools parallelise for free. A flake summary is printed at the end. |
| `--no-clean` | Skip app data clear between flows (not yet implemented) |
| `--no-teardown` | Skip teardown blocks (not yet wired) |
| `--keep-devices` | Keep devices running after completion (not yet wired) |
| `--no-perf` | Disable performance capture |
| `--a11y <off\|critical\|relaxed\|strict>` | Override every flow's accessibility audit level (default `relaxed`). `off` disables; `critical` runs tree checks only; `relaxed` adds opportunistic contrast; `strict` forces a per-block screenshot + AAA bands |
| `--a11y-min-confidence <0.0–1.0>` | Override every flow's `a11y_min_confidence`: drop a11y findings below this confidence. `0` surfaces every heuristic finding, higher keeps only confident ones. Wins over `[flow.options]` and the level default. |
| `--rebuild` | Bypass the persistent install cache for this run (rebuild + reinstall every app on every device). Cache is still written after a successful build, so the next run benefits. |
| `--no-build` | Skip build+install entirely. If the device already has the bundle, golem trusts it and runs flows; if not, the flow fails loudly. The cache is left untouched. Use when iterating on flow files against a known-good binary. |
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

Inspect the live UI element hierarchy from a running device.

```bash
golem tree [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--platform <ios\|android>` | Filter by platform |
| `--device <NAME>` | Filter by device name or UDID (substring match) |
| `--bundle <ID>` | App bundle ID (default: `fail.golem.test`) |
| `--full` | Show full tree without viewport filtering |
| `--json` | Output as JSON |
| `--verbose` | Show metadata: CDP status, enrichment, keyboard, safe area |

## `golem devices`

List all connected simulators, emulators, and physical devices.

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
