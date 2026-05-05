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
      BUILD_START_TS=$(date +%s)
      echo "building Tauri iOS for $DEVICE_ID..." >&2
      # Clear prior build artifacts. The rename step that tauri-cli does
      # at the end of `ios build` fails with "Directory not empty" if the
      # target-arch dir already exists from a prior run — and the failure
      # is silent under `set +e`. When that happens the .app we pick up
      # below is whatever stale tree was left behind, leading to weeks-
      # old bundles being installed without warning. Clearing both the
      # xcarchive and the per-arch output dirs makes the rename succeed
      # every time.
      rm -rf src-tauri/gen/apple/build/*.xcarchive
      rm -rf src-tauri/gen/apple/build/arm64-sim
      rm -rf src-tauri/gen/apple/build/x86_64
      rm -rf src-tauri/gen/apple/build/aarch64
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

    # Find produced .app. Prefer the per-arch target dir (the canonical
    # output) over the xcarchive copy — when the rename succeeded both
    # exist with the same content, but when it failed the xcarchive copy
    # may be a stale or empty shell.
    if [[ "$IS_SIMULATOR" == "1" ]]; then
      HOST_ARCH=$(uname -m)
      if [[ "$HOST_ARCH" == "x86_64" ]]; then
        TARGET_DIR="src-tauri/gen/apple/build/x86_64"
      else
        TARGET_DIR="src-tauri/gen/apple/build/arm64-sim"
      fi
    else
      TARGET_DIR="src-tauri/gen/apple/build/aarch64"
    fi
    APP_PATH=$(find "$TARGET_DIR" -maxdepth 2 -name "*.app" -type d -print -quit 2>/dev/null)
    if [[ -z "$APP_PATH" ]]; then
      # Fall back to a wider search if the per-arch dir is missing.
      APP_PATH=$(find src-tauri/gen/apple/build -maxdepth 5 -name "*.app" -type d -print -quit)
    fi
    if [[ -z "$APP_PATH" || ! -f "$APP_PATH/Info.plist" ]]; then
      echo "error: tauri build failed (exit $TAURI_EXIT) and no valid .app was produced" >&2
      exit 1
    fi
    # Guard against silent stale-bundle installs: if we ran the build
    # (not install-only) the .app must have been written during this run.
    # Picking up a months-old .app because the rename-step failed silently
    # is what bit us for weeks; this turns it into a loud failure.
    if [[ "$MODE" != "install-only" ]]; then
      APP_MTIME=$(stat -f %m "$APP_PATH")
      if (( APP_MTIME < BUILD_START_TS )); then
        echo "error: .app at $APP_PATH was not refreshed by this build (mtime $APP_MTIME < build start $BUILD_START_TS). The tauri-cli rename likely failed and we'd be installing a stale bundle." >&2
        exit 1
      fi
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
