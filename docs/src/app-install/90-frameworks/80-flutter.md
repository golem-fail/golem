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
