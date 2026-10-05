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

- **native-ios** — xcodebuild + `xcrun simctl install` (simulator) / `xcrun devicectl` or `ios-deploy` (physical). Reads these variables from `install_env` or the shell:
  - `CONFIGURATION` and `XCODE_SCHEME`: override the values written into the script.
  - `XCCONFIG`: an `.xcconfig` path, passed as `-xcconfig`.
  - `DEVELOPMENT_TEAM`: your Apple team ID, for a physical device. The script passes it as a build setting with `-allowProvisioningUpdates`. If a physical-device build fails, the error names this variable. **The physical-device path is not yet validated (#59).**

  The script fails if the `.app` is older than the build start.
- **native-android** — `./gradlew :<module>:assemble<Flavor><BuildType>`, then `adb -s <serial> install` of the APK that build wrote. Reads these variables from `install_env` or the shell:
  - `BUILD_TYPE`: the Gradle build type, `debug` by default. A release build needs a `signingConfig`, because Android installs only signed APKs.
  - `FLAVOR`: the product flavor. With more than one flavor dimension, use the full flavor name, for example `freeProd`.

  The script fails if the APK is older than the build start. If `aapt2` is found under `ANDROID_HOME/build-tools`, the script also fails when the APK's `applicationId` is not the app's `bundle`.
- **tauri** — `tauri ios build` / `tauri android build` then install. Detects package manager from lockfiles (`npm` / `yarn` / `pnpm` / `bun` / `cargo tauri`). Reads these variables from `install_env` or the shell:
  - `BUILD_TYPE`: `debug` (default) or `release`. A release build on Android needs a `signingConfig` in `src-tauri/gen/android`, because Android installs only signed APKs. Without one, the script stops with an error that names the signing problem.
  - `TAURI_BUILD_CONFIG`: passed to `tauri … build --config`. It is JSON, or the path to a config file that tauri merges over `tauri.conf.json`.
  - `TAURI_BUILD_FEATURES`: passed to `tauri … build --features`, comma-separated.

  The script installs only an artifact that the current build wrote. It fails if the `.app` or the APK is older than the build start.
- **expo** — Expo / React Native. One cross-platform script honouring `EXPO_BUILD_MODE`:
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

Scripts are plain bash — customise freely after scaffolding. Extend to other frameworks (Flutter, Capacitor, etc.) by hand.

- **capacitor** — Capacitor / Ionic. One cross-platform script:
  1. Install JS dependencies when the lockfile or `package.json` changed.
  2. Run the web build (`npm run build` by default, empty for none). The script fails if no file in `webDir` was written by this build, because `cap sync` would otherwise ship the previous bundle.
  3. Run `cap sync <platform>`.
  4. Build and install with Gradle and `adb` (Android) or `xcodebuild` and `simctl` (iOS). These are the same steps as the native templates, with the same freshness checks.

  The script reads these variables from `install_env` or the shell:
  - `BUILD_TYPE`: `debug` (default) or `release`. It selects the Gradle build type and the Xcode configuration (`Debug` / `Release`).
  - `FLAVOR`, `XCCONFIG`, `DEVELOPMENT_TEAM`, `DERIVED_DATA`: as for native-android and native-ios.

  The `android/` and `ios/` projects are source in a Capacitor app, so the script never creates them. If one is missing, the script fails and names `cap add <platform>`. The iOS build uses `ios/App/App.xcworkspace` when it exists (CocoaPods), else `ios/App/App.xcodeproj` (Swift Package Manager, the Capacitor 8 default), with the scheme `App`.

  **Release builds:** golem reads the page through the web inspector, which Capacitor turns off in release builds. To test a release build, set `webContentsDebuggingEnabled: true` in the Capacitor config.

- **kmp** — Kotlin Multiplatform with Compose Multiplatform. One cross-platform script:
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

- **nativescript** — NativeScript 8+. One cross-platform script: it installs JS dependencies when they changed, runs `ns build <platform>`, then installs the artifact that build wrote with `simctl` / `devicectl` or `adb`. It does not use `ns build --copy-to`, which covers only the device `.ipa` and has been broken for simulator builds. The iOS `.app` is looked up under both `platforms/ios/build/` and `platforms/ios/Build/`, because CLI versions differ.

  **Host needs:** the NativeScript CLI, as a `nativescript` devDependency (run with `npx ns`) or a global `ns`, and CocoaPods + Ruby for iOS.

  The script reads these variables from `install_env` or the shell:
  - `BUILD_TYPE`: `debug` (default) or `release` (adds `--release`).
  - `NS_BUILD_ARGS`: extra `ns build` flags, for example `--env.*` bundler flags, or the `--key-store-*` flags that an Android release build needs for signing.
  - `DEVELOPMENT_TEAM` (`--team-id`) or `PROVISION` (`--provision`): signing for a physical iOS device, which builds with `--for-device`.

  **Selectors:** NativeScript renders real native views, so text selectors work on both platforms. `automationText` (and `testID`) set the accessibility *identifier*. On iOS golem matches it with `accessibility_label`. On Android it is only a view tag, which the accessibility tree does not carry, so golem cannot see it. To give an Android view a label golem can match, set `accessibilityLabel` (Android's `contentDescription`). On iOS that label replaces the view's visible text.
