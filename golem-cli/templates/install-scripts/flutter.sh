#!/usr/bin/env bash
# golem install script — Flutter
#
# Invoked by golem before each flow to build and install a Flutter app onto
# a target simulator/emulator or physical device. Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Builds with `flutter build apk` / `flutter build ios`, then installs the
# artifact that build wrote with adb (Android) or simctl / devicectl (iOS).
# Not `flutter install`: it uninstalls the app first, which wipes its data
# and granted permissions on every install.
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE        = "debug" (default) | "profile" | "release". A simulator
#                       runs debug only; a physical iOS device runs profile or
#                       release only (a debug build needs the debugger to start).
#   FLAVOR            = Flutter flavor (Gradle product flavor / Xcode scheme)
#   DART_DEFINES      = space-separated KEY=VALUE pairs, each passed as
#                       --dart-define (a value cannot contain a space)
#   DART_DEFINE_FILE  = .json or .env file for --dart-define-from-file
#   FLUTTER_BUILD_ARGS = extra `flutter build` flags, word-split
#   DEVELOPMENT_TEAM  = Apple team ID for a physical iOS device
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
FLUTTER_DIR="{{FLUTTER_DIR}}"     # path to the Flutter project (contains pubspec.yaml)
FLUTTER_CMD="{{FLUTTER_CMD}}"     # Flutter CLI: flutter | fvm flutter

BUILD_TYPE="${BUILD_TYPE:-debug}"
FLAVOR="${FLAVOR:-}"
DART_DEFINES="${DART_DEFINES:-}"
DART_DEFINE_FILE="${DART_DEFINE_FILE:-}"
FLUTTER_BUILD_ARGS="${FLUTTER_BUILD_ARGS:-}"
DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"

case "$BUILD_TYPE" in
  debug | profile | release) ;;
  *)
    echo "error: unknown BUILD_TYPE='$BUILD_TYPE' (expected 'debug', 'profile' or 'release')" >&2
    exit 1
    ;;
esac

BUILD_ARGS=("--$BUILD_TYPE")
if [[ -n "$FLAVOR" ]]; then BUILD_ARGS+=(--flavor "$FLAVOR"); fi
for define in $DART_DEFINES; do BUILD_ARGS+=("--dart-define=$define"); done
if [[ -n "$DART_DEFINE_FILE" ]]; then BUILD_ARGS+=("--dart-define-from-file=$DART_DEFINE_FILE"); fi
# Word-split on purpose: FLUTTER_BUILD_ARGS is a list of flags.
# shellcheck disable=SC2206
if [[ -n "$FLUTTER_BUILD_ARGS" ]]; then BUILD_ARGS+=($FLUTTER_BUILD_ARGS); fi

cd "$FLUTTER_DIR"

{{>helpers}}

{{>ios}}

case "$PLATFORM" in
  android)
    # The Gradle output, laid out by variant (`debug`, `free/release`). The
    # `flutter-apk/` copy has no variant directories to narrow a pick by.
    APK_ROOT="build/app/outputs/apk"
    VARIANT="$BUILD_TYPE"
    if [[ -n "$FLAVOR" ]]; then VARIANT="$FLAVOR/$BUILD_TYPE"; fi
    if [[ "$MODE" != "install-only" ]]; then
      BUILD_START_TS=$(date +%s)
      # An up-to-date build leaves the APK unwritten, which the freshness
      # guard cannot tell from a stale one. With the APK gone, Gradle
      # repackages it.
      rm -rf "${APK_ROOT:?}/$VARIANT"
      echo "flutter build apk ($VARIANT)..." >&2
      $FLUTTER_CMD build apk "${BUILD_ARGS[@]}" 1>&2
    else
      echo "install-only: reusing prior APK for $DEVICE_ID" >&2
    fi
    APK=$(golem_pick_apk "$APK_ROOT" "$VARIANT")
    if [[ -z "$APK" ]]; then
      golem_no_apk_error "$APK_ROOT"
      exit 1
    fi
    if [[ "$MODE" != "install-only" ]]; then
      golem_require_fresh "$APK" "$BUILD_START_TS" || exit 1
    fi
    echo "installing $APK on $DEVICE_ID..." >&2
    adb -s "$DEVICE_ID" install -r "$APK" 1>&2
    ;;
  ios)
    if golem_is_simulator "$DEVICE_ID"; then
      if [[ "$BUILD_TYPE" != debug ]]; then
        echo "error: Flutter builds only debug for a simulator (BUILD_TYPE='$BUILD_TYPE')" >&2
        exit 1
      fi
      PRODUCTS_DIR="build/ios/iphonesimulator"
      BUILD_ARGS+=(--simulator)
    else
      if [[ "$BUILD_TYPE" == debug ]]; then
        echo "error: a debug Flutter build starts on a physical iOS device only under the debugger;" >&2
        echo "       set BUILD_TYPE=profile or BUILD_TYPE=release" >&2
        exit 1
      fi
      PRODUCTS_DIR="build/ios/iphoneos"
      # Flutter passes FLUTTER_XCODE_<setting> to xcodebuild as <setting>.
      if [[ -n "$DEVELOPMENT_TEAM" ]]; then export FLUTTER_XCODE_DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM"; fi
    fi
    if [[ "$MODE" != "install-only" ]]; then
      BUILD_START_TS=$(date +%s)
      rm -rf "$PRODUCTS_DIR"/*.app
      echo "flutter build ios ($BUILD_TYPE, $PRODUCTS_DIR)..." >&2
      $FLUTTER_CMD build ios "${BUILD_ARGS[@]}" 1>&2
    else
      echo "install-only: reusing prior build for $DEVICE_ID" >&2
    fi
    APP_PATH=$(golem_pick_app "$PRODUCTS_DIR")
    if [[ -z "$APP_PATH" ]]; then
      echo "error: no .app under $PRODUCTS_DIR (build may have been skipped — re-run without install-only)" >&2
      exit 1
    fi
    if [[ "$MODE" != "install-only" ]]; then
      golem_require_fresh "$APP_PATH" "$BUILD_START_TS" || exit 1
    fi
    golem_ios_install_app "$DEVICE_ID" "$APP_PATH"
    ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
