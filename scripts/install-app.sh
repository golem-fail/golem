#!/usr/bin/env bash
# golem install script — Tauri mobile
#
# Invoked by golem before each flow to build and install a Tauri mobile
# app onto a target simulator/emulator or physical device.
# Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default)
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
TAURI_DIR="test-app"               # path to Tauri project (contains src-tauri/)
IOS_SCHEME="app_iOS"             # iOS scheme name
TAURI_CMD="cargo tauri"               # tauri CLI runner (npx/yarn/pnpm/bun/cargo tauri)

cd "$TAURI_DIR"

case "$PLATFORM" in
  ios)
    # Detect simulator vs physical device.
    IS_SIMULATOR=0
    if xcrun simctl list devices --json 2>/dev/null | grep -q "\"$DEVICE_ID\""; then
      IS_SIMULATOR=1
    fi

    if [[ "$MODE" != "install-only" ]]; then
      echo "building Tauri iOS for $DEVICE_ID..." >&2
      # Clear prior xcarchive to avoid stale-state conflicts.
      rm -rf src-tauri/gen/apple/build/*.xcarchive
      # Known bug: tauri-cli 2.10 + Xcode 26 exits nonzero on a post-archive
      # rename step even after producing a valid signed .app. We tolerate
      # nonzero exit here; the presence+validity check below is the gate.
      set +e
      if [[ "$IS_SIMULATOR" == "1" ]]; then
        HOST_ARCH=$(uname -m)
        if [[ "$HOST_ARCH" == "x86_64" ]]; then
          $TAURI_CMD ios build --debug --target x86_64 1>&2
        else
          $TAURI_CMD ios build --debug --target aarch64-sim 1>&2
        fi
      else
        $TAURI_CMD ios build --debug --target aarch64 1>&2
      fi
      TAURI_EXIT=$?
      set -e
    else
      echo "install-only: reusing prior build for $DEVICE_ID" >&2
      TAURI_EXIT=0
    fi

    # Find produced .app
    APP_PATH=$(find src-tauri/gen/apple/build -maxdepth 5 -name "*.app" -type d -print -quit)
    if [[ -z "$APP_PATH" || ! -f "$APP_PATH/Info.plist" ]]; then
      echo "error: tauri build failed (exit $TAURI_EXIT) and no valid .app was produced" >&2
      exit 1
    fi
    if [[ "$TAURI_EXIT" -ne 0 ]]; then
      echo "warning: tauri exited $TAURI_EXIT but .app was built; proceeding to install" >&2
    fi

    if [[ "$IS_SIMULATOR" == "1" ]]; then
      xcrun simctl install "$DEVICE_ID" "$APP_PATH" 1>&2
    elif xcrun devicectl --version >/dev/null 2>&1; then
      xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH" 1>&2
    elif command -v ios-deploy >/dev/null 2>&1; then
      ios-deploy --id "$DEVICE_ID" --bundle "$APP_PATH" --no-wifi 1>&2
    else
      echo "error: need Xcode 15+ (devicectl) or ios-deploy for physical devices" >&2
      exit 1
    fi
    ;;
  android)
    if [[ "$MODE" != "install-only" ]]; then
      echo "building Tauri Android..." >&2
      # Tauri produces a universal APK by default; build without installing.
      $TAURI_CMD android build --debug --apk 1>&2
    else
      echo "install-only: reusing prior APK for $DEVICE_ID" >&2
    fi

    # Find produced APK
    APK=$(find src-tauri/gen/android/app/build/outputs/apk -name "*.apk" -print -quit)
    if [[ -z "$APK" ]]; then
      echo "error: no APK found (build may have been skipped — re-run without install-only)" >&2
      exit 1
    fi
    adb -s "$DEVICE_ID" install -r "$APK" 1>&2
    ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
