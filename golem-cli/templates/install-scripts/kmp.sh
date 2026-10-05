#!/usr/bin/env bash
# golem install script — Kotlin Multiplatform (Compose Multiplatform)
#
# Invoked by golem before each flow to build and install a Kotlin
# Multiplatform app onto a target simulator/emulator or physical device.
# Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Android builds the app module with Gradle and installs the APK with adb.
# iOS builds the Xcode project, whose "Run Script" phase builds the Kotlin
# framework (`./gradlew :<module>:embedAndSignAppleFrameworkForXcode`). The
# script checks that the Kotlin code is really in the built .app before it
# installs it.
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE       = "debug" (default) | "release". Selects the Gradle build
#                      type and the Xcode configuration (Debug / Release).
#                      A release build needs signing on Android.
#   FLAVOR           = Gradle product flavor (default: none)
#   XCODE_SCHEME     = overrides the scheme below
#   XCCONFIG         = path to an .xcconfig, passed as `xcodebuild -xcconfig`
#   DEVELOPMENT_TEAM = Apple team ID, for a physical iOS device
#   DERIVED_DATA     = xcodebuild derived-data dir (default ./build/DerivedData)
#   ORG_GRADLE_PROJECT_<name> = Gradle reads these as `-P<name>=…` itself,
#                      for both platforms' builds.
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
KMP_DIR="{{KMP_DIR}}"                 # project root (contains settings.gradle.kts)
ANDROID_MODULE="{{ANDROID_MODULE}}"   # Android app module: composeApp | androidApp
IOS_DIR="{{IOS_DIR}}"                 # Xcode app directory, relative to KMP_DIR (e.g. iosApp)
XCODE_SCHEME="${XCODE_SCHEME:-{{XCODE_SCHEME}}}"   # Xcode scheme name

BUILD_TYPE="${BUILD_TYPE:-debug}"
FLAVOR="${FLAVOR:-}"
XCCONFIG="${XCCONFIG:-}"
DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"
DERIVED_DATA="${DERIVED_DATA:-./build/DerivedData}"

case "$BUILD_TYPE" in
  debug)   XCODE_CONFIGURATION=Debug ;;
  release) XCODE_CONFIGURATION=Release ;;
  *)
    echo "error: unknown BUILD_TYPE='$BUILD_TYPE' (expected 'debug' or 'release')" >&2
    exit 1
    ;;
esac

cd "$KMP_DIR"

{{>helpers}}

{{>android}}

{{>ios}}

case "$PLATFORM" in
  android)
    golem_android_build_install . "$ANDROID_MODULE" "$FLAVOR" "$BUILD_TYPE" \
      "$DEVICE_ID" "$BUNDLE_ID" "$MODE"
    ;;
  ios)
    # The CocoaPods integration (`kotlin("native.cocoapods")`) builds through
    # a workspace and needs `pod install`; direct integration has only the
    # .xcodeproj.
    XCODE_PROJECT=$(golem_newest "$IOS_DIR" -maxdepth 1 -name '*.xcworkspace' -type d)
    if [[ -n "$XCODE_PROJECT" ]]; then
      # Pods/Manifest.lock is CocoaPods' own record of what it installed; it
      # differs from Podfile.lock when the Podfile moved on since.
      if [[ "$MODE" != "install-only" ]] \
        && ! cmp -s "$IOS_DIR/Podfile.lock" "$IOS_DIR/Pods/Manifest.lock"; then
        echo "pod install ($IOS_DIR)..." >&2
        ( cd "$IOS_DIR" && pod install ) 1>&2
      fi
    else
      XCODE_PROJECT=$(golem_newest "$IOS_DIR" -maxdepth 1 -name '*.xcodeproj' -type d)
    fi
    if [[ -z "$XCODE_PROJECT" ]]; then
      echo "error: no .xcodeproj or .xcworkspace in $KMP_DIR/$IOS_DIR" >&2
      exit 1
    fi

    APP_PATH=$(golem_ios_build "$XCODE_PROJECT" "$XCODE_SCHEME" "$XCODE_CONFIGURATION" \
      "$XCCONFIG" "$DEVELOPMENT_TEAM" "$DERIVED_DATA" "$DEVICE_ID" "$MODE")

    # xcodebuild succeeds without the Kotlin framework when the project lacks
    # the step that builds it, and the .app then fails at launch with nothing
    # pointing here. `Konan_` is the Kotlin/Native runtime's symbol prefix:
    # it is in the binary that holds the Kotlin code, wherever that is (the
    # executable, a debug dylib, or a dynamic framework).
    if ! grep -r -a -q "Konan_" "$APP_PATH"; then
      echo "error: $APP_PATH contains no Kotlin code. Check both of these:" >&2
      echo "       1. The iOS target in the shared module's build.gradle.kts declares" >&2
      echo "          binaries.framework { … }." >&2
      echo "       2. The Xcode target has a Run Script phase, before Compile Sources," >&2
      echo "          that runs ./gradlew :<shared module>:embedAndSignAppleFrameworkForXcode." >&2
      exit 1
    fi

    golem_ios_install_app "$DEVICE_ID" "$APP_PATH"
    ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
