#!/usr/bin/env bash
# golem install script — Capacitor / Ionic
#
# Invoked by golem before each flow to build and install a Capacitor app onto
# a target simulator/emulator or physical device. Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Steps: install JS dependencies when they changed, build the web app, copy
# it into the native project with `cap sync`, then build and install with
# Gradle (Android) or xcodebuild (iOS). The native projects (android/, ios/)
# are source in a Capacitor app, so this script never creates them.
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE       = "debug" (default) | "release". Selects the Gradle build
#                      type and the Xcode configuration (Debug / Release).
#                      A release build needs signing on Android, and
#                      `webContentsDebuggingEnabled: true` in the Capacitor
#                      config on both platforms: golem reads the page through
#                      the web inspector, which release builds turn off.
#   FLAVOR           = Gradle product flavor (default: none)
#   XCCONFIG         = path to an .xcconfig, passed as `xcodebuild -xcconfig`
#   DEVELOPMENT_TEAM = Apple team ID, for a physical iOS device
#   DERIVED_DATA     = xcodebuild derived-data dir (default ./build/DerivedData)
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
CAP_DIR="{{CAP_DIR}}"             # path to the Capacitor project (contains capacitor.config.*)
CAP_CMD="{{CAP_CMD}}"             # Capacitor CLI runner: npx cap | yarn cap | pnpm cap | bunx cap
PM_INSTALL="{{PM_INSTALL}}"       # dependency install: npm install | yarn | pnpm install | bun install
WEB_BUILD="{{WEB_BUILD}}"         # web build command (e.g. npm run build); empty = no build step
WEB_DIR="{{WEB_DIR}}"             # `webDir` from the Capacitor config

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

cd "$CAP_DIR"

{{>helpers}}

{{>deps}}

{{>android}}

{{>ios}}

case "$PLATFORM" in
  ios|android) ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

if [[ ! -d "$PLATFORM" ]]; then
  echo "error: no $PLATFORM/ project in $CAP_DIR. Create it once with" >&2
  echo "       '$CAP_CMD add $PLATFORM' and commit it: it is source in a Capacitor app." >&2
  exit 1
fi

if [[ "$MODE" != "install-only" ]]; then
  golem_ensure_deps "$PM_INSTALL"

  if [[ -n "$WEB_BUILD" ]]; then
    WEB_BUILD_START=$(date +%s)
    echo "building the web app ($WEB_BUILD)..." >&2
    $WEB_BUILD 1>&2
    # `cap sync` copies whatever is in webDir. A web build that quietly did
    # nothing would ship the previous bundle inside a fresh native build.
    WEB_NEWEST=$(golem_newest "$WEB_DIR" -type f)
    if [[ -z "$WEB_NEWEST" ]]; then
      echo "error: $WEB_DIR is empty after '$WEB_BUILD'. Is WEB_DIR the webDir of the Capacitor config?" >&2
      exit 1
    fi
    golem_require_fresh "$WEB_NEWEST" "$WEB_BUILD_START"
  fi

  echo "cap sync $PLATFORM..." >&2
  $CAP_CMD sync "$PLATFORM" 1>&2
fi

case "$PLATFORM" in
  android)
    golem_android_build_install android app "$FLAVOR" "$BUILD_TYPE" \
      "$DEVICE_ID" "$BUNDLE_ID" "$MODE"
    ;;
  ios)
    # CocoaPods projects build through the workspace; Swift Package Manager
    # projects (the Capacitor 8 default) have only the .xcodeproj.
    XCODE_PROJECT="ios/App/App.xcodeproj"
    if [[ -d ios/App/App.xcworkspace ]]; then XCODE_PROJECT="ios/App/App.xcworkspace"; fi
    golem_ios_build_install "$XCODE_PROJECT" App "$XCODE_CONFIGURATION" "$XCCONFIG" \
      "$DEVELOPMENT_TEAM" "$DERIVED_DATA" "$DEVICE_ID" "$MODE"
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
