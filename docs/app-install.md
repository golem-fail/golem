<!-- Generated from docs/src/app-install/ — edit the parts there, then run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs`. -->
# App Install

*Putting the clay on the device before the word reaches it.*

← [Back to README](../README.md) · See also [CLI Reference](cli-reference.md) · [Test Structure](test-structure.md)

By default golem assumes the app under test is already installed on the target device. For fresh simulators, CI pipelines, or teams that want per-test builds, you can supply an install script that golem runs before each flow.

golem runs the script with four arguments: `$1` platform, `$2` device id, `$3` bundle id and `$4` mode (see [Script contract](#script-contract)). `golem install-script` writes one from a template. Without a terminal, `golem install-script --print <framework>` writes a template to stdout.

## Contents

- [Quick start](#quick-start)
- [Project-level `[[apps]]` registry](#project-level-apps-registry)
- [`install_script` forms](#install_script-forms)
- [Script contract](#script-contract)
  - [Environment](#environment)
- [Parameterizing installs: `install_env` + profiles](#parameterizing-installs-install_env--profiles)
  - [`install_env`](#install_env--inject-environment-values)
  - [Profiles](#profiles--select-app-definitions-by---profile)
- [Install cache](#install-cache)
- [Cache flags: `--rebuild` and `--no-build`](#cache-flags---rebuild-and---no-build)
- [When no `install_script` is configured](#when-no-install_script-is-configured)
- [Frameworks the scaffold supports](#frameworks-the-scaffold-supports)
  - [native-ios](#native-ios)
  - [native-android](#native-android)
  - [tauri](#tauri)
  - [expo](#expo)
  - [capacitor](#capacitor)
  - [kmp](#kmp)
  - [nativescript](#nativescript)
  - [flutter](#flutter)

## Quick start
<!-- summary: golem install-script, and --print / --template without a terminal -->

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

With `--template`, the command asks nothing. Each value that no flag gives is the default its prompt would show, detected from the project as the prompt detects it. golem adds the script to the app's `[[apps]]` entry in `golem.toml`, if there is one. `--app` is the app's name in `[[apps]]` (default `app`). `--path` is the app's project directory. For `native-ios`, `--path` is the `.xcodeproj` or `.xcworkspace`, or a directory that has one. The templates are in [Frameworks the scaffold supports](#frameworks-the-scaffold-supports).

**Monorepos:** a repository can have many apps. Run the command once for each app, with its own `--app` and `--path`, or loop over the `[[apps]]` of `golem.toml`:

```bash
golem install-script --template expo --app requester --path apps/requester-app
golem install-script --template expo --app provider  --path apps/provider-app
```

## Project-level `[[apps]]` registry
<!-- summary: Declare each app once in golem.toml [[apps]]; flows inherit it -->

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
<!-- summary: One script for both platforms, or one for each platform -->

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
<!-- summary: The arguments: $1 platform, $2 device id, $3 bundle id, $4 mode -->

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
<!-- summary: install_env values and --profile app definitions -->

One script can cover staging and production, Debug and Release, or a local and a cloud build. Two settings do this: `install_env` and profiles.

### `install_env` — inject environment values

golem sets these variables in the install script's environment:

```toml
[[apps]]
name = "app"
bundle = "com.example.app"
install_script = "scripts/install.sh"
install_env = { APP_ENV = "staging", SANDBOX_ID = "${sandbox_id}" }
```

`${var}` resolves at install time, so `golem run --var sandbox_id=12345` sets `SANDBOX_ID` to `12345`. The install runs before any step, so only these resolve: `--var`, project and flow vars, and the builtins `_platform`, `_udid` and `_app`. Any other variable fails the install, and the error names it.

### Profiles — select app *definitions* by `--profile`

Several `[[apps]]` or `[[flow.apps]]` entries can share a `name` if each has its own `profile`. An entry without `profile` is the default. `golem run --profile <name>` uses, for each app, the entry with that profile, else the default.

```toml
# golem run: local build.
[[apps]]
name = "app-e"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"

# golem run --profile eas: the same script, a cloud build.
[[apps]]
name = "app-e"
profile = "eas"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"
install_env = { EXPO_BUILD_MODE = "eas", EAS_PROFILE = "preview" }
```

- A flow entry merges over the project entry with the same name and profile; the flow wins. Entries of different profiles share nothing, so repeat fields such as `bundle`.
- The default as a fallback is not an error. It is an error only when an app has neither its profile's entry nor a default. A `--profile` that no app has gets a warning: it is likely a typo.
- The script does not see the profile. The profile only picks the entry, and so its `install_script` and `install_env`.

## Install cache
<!-- summary: golem installs once per app and device, and when it installs again -->

**In one run,** golem runs an app's install script at most once per device. After a failed install, the other flows for that app and device skip, with the reason.

**Across runs,** `.golem/install-cache.json` remembers each install. A later `golem run` skips the build and the install when all three checks pass:

1. The app is on the device (`simctl get_app_container`, `adb shell pm path`).
2. The app's install time on the device is the cached one. A reinstall by another tool (Xcode Run, `simctl install`) fails this check.
3. The source fingerprint is the cached one: `git rev-parse HEAD` plus a hash of `git status --porcelain`, or without git, a hash of the files that `.gitignore` does not exclude.

A hit prints `skipped (cache hit)`. A miss builds and installs, and prints why:

- `source fingerprint changed (git:abc → git:def)`: a commit or an edit. A dirty tree shows a suffix, `git:abc1234+0a1b`.
- `device install-time differs`: another tool reinstalled the app.
- `bundle no longer installed on device`: the simulator was reset, or the app was removed.
- `fingerprint unavailable`: golem could not compute one.

A first install, with no cache entry, prints no reason.

## Cache flags: `--rebuild` and `--no-build`
<!-- summary: --rebuild forces an install; --no-build skips it -->

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
<!-- summary: Without install_script, golem launches the installed app -->

If an app has no `install_script` field in `golem.toml` or its flow file, golem behaves like a permanent `--no-build` for that app: no install runs, the flow goes straight to `launch`. The two paths converge on the success case but differ on the failure mode:

| Scenario | App present | App absent |
|---|---|---|
| **No `install_script` in TOML** | launches, runs flow | `launch` errors at runtime, cache marks `FailedNoScript`, subsequent flows skip with that reason |
| **`--no-build` flag** | mark `Succeeded` upfront, runs flow | flow fails immediately with an actionable hint to drop the flag |

So if your project has no install scripts anywhere, `--no-build` is redundant — golem already assumes the apps are preinstalled. The flag earns its keep when scripts *are* configured and you want to bypass them temporarily.

## Frameworks the scaffold supports
<!-- summary: The templates golem install-script writes, one per framework -->

`golem install-script` has a template for each of these frameworks. `golem install-script --print <framework>` writes one to stdout. A script is plain bash: change it freely after you scaffold it. For another framework, write the script by hand, to the [script contract](#script-contract).

### native-ios
<!-- summary: xcodebuild, then simctl or devicectl install -->

xcodebuild + `xcrun simctl install` (simulator) / `xcrun devicectl` or `ios-deploy` (physical). Reads these variables from `install_env` or the shell:
- `CONFIGURATION` and `XCODE_SCHEME`: override the values written into the script.
- `XCCONFIG`: an `.xcconfig` path, passed as `-xcconfig`.
- `DEVELOPMENT_TEAM`: your Apple team ID, for a physical device. The script passes it as a build setting with `-allowProvisioningUpdates`. If a physical-device build fails, the error names this variable. **The physical-device path is not yet validated (#59).**

The script fails if the `.app` is older than the build start.

### native-android
<!-- summary: Gradle assemble, then adb install -->

`./gradlew :<module>:assemble<Flavor><BuildType>`, then `adb -s <serial> install` of the APK that build wrote. Reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: the Gradle build type, `debug` by default. A release build needs a `signingConfig`, because Android installs only signed APKs.
- `FLAVOR`: the product flavor. With more than one flavor dimension, use the full flavor name, for example `freeProd`.

The script fails if the APK is older than the build start. If `aapt2` is found under `ANDROID_HOME/build-tools`, the script also fails when the APK's `applicationId` is not the app's `bundle`.

### tauri
<!-- summary: tauri ios/android build, then install -->

`tauri ios build` / `tauri android build` then install. Detects package manager from lockfiles (`npm` / `yarn` / `pnpm` / `bun` / `cargo tauri`). Reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release`. A release build on Android needs a `signingConfig` in `src-tauri/gen/android`, because Android installs only signed APKs. Without one, the script stops with an error that names the signing problem.
- `TAURI_BUILD_CONFIG`: passed to `tauri … build --config`. It is JSON, or the path to a config file that tauri merges over `tauri.conf.json`.
- `TAURI_BUILD_FEATURES`: passed to `tauri … build --features`, comma-separated.

The script installs only an artifact that the current build wrote. It fails if the `.app` or the APK is older than the build start.

### expo
<!-- summary: Expo prebuild and a local Release build, or EAS; golem run --dev -->

Expo / React Native, one script for both platforms. `EXPO_BUILD_MODE` picks the build:

- `local` (default): `expo prebuild`, then a **Release** native build, then `simctl` or `adb` install. The build embeds the JS bundle, so the app runs without Metro. It needs no Expo account and no signing account on a simulator or emulator.
- `eas`: an EAS cloud build, downloaded and installed. It uses the latest finished build unless `GOLEM_REBUILD=1`, and needs `EXPO_TOKEN`. **golem's own CI does not test this mode**, so try it on your project first.

The script finds the package manager from the lockfile, and the iOS scheme from the generated project.

The template is for Expo only: it needs `expo prebuild`. A **bare React Native** project (no `expo` dependency) commits its own `ios/` and `android/`; use **native-ios** and **native-android** for it.

**Iterating on JS: `golem run --dev`.** A Release build embeds the bundle, so each JS change needs a native rebuild. Instead:

1. Build and install a Debug build once: `npx expo run:ios` or `npx expo run:android`.
2. Keep `npx expo start` running.
3. Run `golem run --dev`. golem skips the build and the install. Each flow relaunches the app, which loads the current bundle from the dev server.

If the dev server does not answer, golem fails with `H503`. Use `--dev` for local work only: a CI run should test the artifact that ships. It works on simulators and emulators; a physical device also needs `adb reverse tcp:8081 tcp:8081` or a LAN host that the device can reach.

**Broken JS fails with `A501`, not as a failing test:**

- Before any flow, `--dev` asks the dev server for the bundle. A build error stops the run and quotes it (`SyntaxError: App.tsx: Unexpected token (9:15)`). On iOS this is the only signal, because the error overlay shows nothing golem can read.
- A step that fails while React Native's error overlay shows reports `A501` with the overlay's message. This covers the native redbox (the bundle never ran; Android only, an iOS simulator stays blank) and the JS LogBox (the app threw after it mounted; both platforms, with the source position, `boom at App.tsx (35:43)`).

golem ignores a LogBox that is minimized to its badge, or that shows a console warning or error: the app under it still works.

### capacitor
<!-- summary: Web build, cap sync, then a native build and install -->

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

### kmp
<!-- summary: Kotlin Multiplatform with Compose Multiplatform -->

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

For element types, `testTag`, bounds and merged semantics, see [Selectors → Canvas-rendered UI](selectors.md#canvas-rendered-ui-compose-compose-multiplatform-flutter).

### nativescript
<!-- summary: ns build, then install the artifact -->

NativeScript 8+. One cross-platform script: it installs JS dependencies when they changed, runs `ns build <platform>`, then installs the artifact that build wrote with `simctl` / `devicectl` or `adb`. It does not use `ns build --copy-to`, which covers only the device `.ipa` and has been broken for simulator builds. The iOS `.app` is looked up under both `platforms/ios/build/` and `platforms/ios/Build/`, because CLI versions differ.

**Host needs:** the NativeScript CLI, as a `nativescript` devDependency (run with `npx ns`) or a global `ns`, and CocoaPods + Ruby for iOS.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release` (adds `--release`).
- `NS_BUILD_ARGS`: extra `ns build` flags, for example `--env.*` bundler flags, or the `--key-store-*` flags that an Android release build needs for signing.
- `DEVELOPMENT_TEAM` (`--team-id`) or `PROVISION` (`--provision`): signing for a physical iOS device, which builds with `--for-device`.

**Selectors:** NativeScript renders real native views, so text selectors work on both platforms. `automationText` (and `testID`) set the accessibility *identifier*. On iOS golem matches it with `accessibility_label`. On Android it is only a view tag, not a `resource-id`, so the accessibility tree does not carry it and golem cannot see it. To give an Android view a label golem can match, set `accessibilityLabel` (Android's `contentDescription`). On iOS that label replaces the view's visible text.

### flutter
<!-- summary: flutter build, then adb or simctl install -->

Flutter 3.19+. One cross-platform script: it runs `flutter build apk` or `flutter build ios`, then installs the artifact that build wrote with `adb` or `simctl` / `devicectl`. It does not use `flutter install`, which uninstalls the app first and so wipes its data and granted permissions on every install. The scaffold detects a `pubspec.yaml` with an `sdk: flutter` dependency, and suggests `fvm flutter` when the project pins its SDK with FVM.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default), `profile` or `release`. A simulator runs `debug` only. A physical iOS device runs `profile` or `release` only, because a debug build starts there only under the debugger.
- `FLAVOR`: the Flutter flavor (`--flavor`), that is, a Gradle product flavor and an Xcode scheme.
- `DART_DEFINES`: space-separated `KEY=VALUE` pairs, each passed as `--dart-define`. A value cannot contain a space. For such values, use `DART_DEFINE_FILE`.
- `DART_DEFINE_FILE`: a `.json` or `.env` file for `--dart-define-from-file`.
- `FLUTTER_BUILD_ARGS`: extra `flutter build` flags, for example `--obfuscate --split-debug-info=<dir>`. Do not pass `--split-per-abi`: the script installs one APK.
- `DEVELOPMENT_TEAM`: the Apple team ID for a physical iOS device. Flutter hands it to xcodebuild as `FLUTTER_XCODE_DEVELOPMENT_TEAM`.

**Selectors:** Flutter draws into one canvas view and copies its semantics tree into the platform accessibility tree. Text selectors work on both platforms. `Semantics(identifier:)` is the identifier that `on_accessibility_label` matches. `MergeSemantics` joins its children's text into one node and removes the children. For element types, merged semantics and bounds, see [Selectors → Canvas-rendered UI](selectors.md#canvas-rendered-ui-compose-compose-multiplatform-flutter).
