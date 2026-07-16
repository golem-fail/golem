# App Install

*Putting the clay on the device before the word reaches it.*

← [Back to README](../README.md) · See also [CLI Reference](cli-reference.md) · [Test Structure](test-structure.md)

By default golem assumes the app under test is already installed on the target device. For fresh simulators, CI pipelines, or teams that want per-test builds, you can supply an install script that golem runs before each flow.

## Contents

- [Quick start](#quick-start)
- [Project-level `[[apps]]` registry](#project-level-apps-registry)
- [`install_script` forms](#install_script-forms)
- [Script contract](#script-contract)
- [Parameterizing installs: `install_env` + profiles](#parameterizing-installs-install_env--profiles)
- [Install cache](#install-cache)
- [Cache flags: `--rebuild` and `--no-build`](#cache-flags---rebuild-and---no-build)
- [When no `install_script` is configured](#when-no-install_script-is-configured)
- [Frameworks the scaffold supports](#frameworks-the-scaffold-supports)

## Quick start

```bash
golem install-script      # interactive: choose framework, answer prompts
```

The scaffold writes `scripts/install-<app>-<platform>.sh` (for native iOS/Android) or `scripts/install-<app>.sh` (for cross-platform frameworks like Tauri and Expo) and can auto-update `golem.toml`.

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

## When no `install_script` is configured

If an app has no `install_script` field in `golem.toml` or its flow file, golem behaves like a permanent `--no-build` for that app: no install runs, the flow goes straight to `launch`. The two paths converge on the success case but differ on the failure mode:

| Scenario | App present | App absent |
|---|---|---|
| **No `install_script` in TOML** | launches, runs flow | `launch` errors at runtime, cache marks `FailedNoScript`, subsequent flows skip with that reason |
| **`--no-build` flag** | mark `Succeeded` upfront, runs flow | flow fails immediately with an actionable hint to drop the flag |

So if your project has no install scripts anywhere, `--no-build` is redundant — golem already assumes the apps are preinstalled. The flag earns its keep when scripts *are* configured and you want to bypass them temporarily.

## Frameworks the scaffold supports

- **native-ios** — xcodebuild + `xcrun simctl install` (simulator) / `xcrun devicectl` or `ios-deploy` (physical)
- **native-android** — `./gradlew :<module>:installDebug` (routes to any connected device via `ANDROID_SERIAL`)
- **tauri** — `tauri ios build` / `tauri android build` then install. Detects package manager from lockfiles (`npm` / `yarn` / `pnpm` / `bun` / `cargo tauri`).
- **expo** — Expo / React Native. One cross-platform script honouring `EXPO_BUILD_MODE`:
  - `local` (default): `expo prebuild` + a **Release** native build (embeds the JS bundle → runs offline, no Metro), then `simctl` / `adb` install. Fully local — no Expo account, works on a simulator/emulator with no signing account.
  - `eas`: build in the cloud via EAS, download the artifact, install it. Reuses the latest finished build unless `GOLEM_REBUILD=1`. Needs `EXPO_TOKEN`. **Written but unverified in golem's own CI** (no account) — validate against a real project before relying on it.

  Detects package manager from lockfiles; auto-discovers the iOS scheme from the generated app project when not set.

  Expo (managed) only — it relies on `expo prebuild` to generate the native projects. **Bare React Native** (a `@react-native-community/cli` project with no `expo` dependency) commits its own `ios/` + `android/` and has no `prebuild`; use **native-ios** + **native-android** for those.

Scripts are plain bash — customise freely after scaffolding. Extend to other frameworks (Flutter, Capacitor, etc.) by hand.
