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
