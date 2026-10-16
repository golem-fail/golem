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
