# test-app-k — Kotlin Multiplatform counter

Minimal Kotlin Multiplatform app (Compose Multiplatform 1.11) that exercises
golem's **kmp install-script** template
(`golem-cli/templates/install-scripts/kmp.sh`).

- **Bundle id:** `fail.golem.testk` (`androidApp/build.gradle.kts` →
  `applicationId`, `iosApp/Configuration/Config.xcconfig` → `BUNDLE_ID`). Keep
  it in sync with `golem.toml`.
- **UI, both iOS layers:** on iOS, a native SwiftUI `Home` screen opens a
  Compose Multiplatform `Compose Counter` screen, so a flow crosses the
  boundary between them. On Android both screens are Compose.
- **Layout:** `shared` (Kotlin Multiplatform, the counter and Android's
  home screen), `androidApp` (the Android app module), `iosApp` (the Xcode
  project, whose Run Script phase builds the `Shared` framework with
  `embedAndSignAppleFrameworkForXcode`). The Xcode project is committed, as in
  a real KMP app, so `scripts/install-app-k.sh` is the rendered template with
  no wrapper.

## Build

```
# from the repo root, via golem:
cargo run -- run e2e/kmp_lifecycle.test.toml --platform ios|android
```

The Gradle wrapper, the Xcode project and the app icons come from JetBrains'
[KMP-App-Template](https://github.com/Kotlin/KMP-App-Template) (Apache-2.0).
