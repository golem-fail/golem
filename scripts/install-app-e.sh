#!/usr/bin/env bash
# golem install script — Expo / React Native (local build + EAS cloud)
#
# Invoked by golem before each flow to build and install an Expo app onto a
# target simulator/emulator or physical device. Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Environment:
#   Template config (you set these — via [[apps]] install_env or the shell):
#     EXPO_BUILD_MODE = "local" (default) | "eas"
#         local: `expo prebuild` + a Release native build (embeds the JS bundle,
#                so the app runs offline with no Metro). Fully local, no account.
#         eas:   build in the cloud via EAS, download the artifact, install it.
#     EAS_PROFILE     = EAS build profile for cloud builds (default "preview")
#     EXPO_TOKEN      = required when EXPO_BUILD_MODE=eas (non-interactive auth)
#     DERIVED_DATA    = iOS xcodebuild derived-data dir (default ./build/DerivedData)
#   golem builtins (golem injects these; the GOLEM_ prefix is reserved for them):
#     GOLEM_REBUILD   = "1" under `golem run --rebuild`; the EAS branch forces a
#                       fresh build then instead of reusing the latest one.
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
EXPO_DIR="test-app-e"           # path to the Expo project (contains app.json)
PM_RUNNER="npx expo"         # expo CLI runner: npx expo | yarn expo | pnpm expo | bunx expo
PM_INSTALL="npm install"       # dependency install: npm install | yarn | pnpm install | bun install
IOS_SCHEME=""       # iOS scheme (Expo names it after the app)

BUILD_MODE="${EXPO_BUILD_MODE:-local}"
EAS_PROFILE="${EAS_PROFILE:-preview}"
GOLEM_REBUILD="${GOLEM_REBUILD:-0}"
DERIVED_DATA="${DERIVED_DATA:-./build/DerivedData}"

cd "$EXPO_DIR"

# ── shared install helpers ──────────────────────────────────────────
install_ios_artifact() {
  local app="$1"
  if [[ -z "$app" || ! -d "$app" ]]; then
    echo "error: no .app to install (build may have been skipped — re-run without install-only)" >&2
    exit 1
  fi
  if xcrun simctl list devices --json 2>/dev/null | grep -q "\"$DEVICE_ID\""; then
    xcrun simctl install "$DEVICE_ID" "$app" 1>&2
  elif xcrun devicectl --version >/dev/null 2>&1; then
    xcrun devicectl device install app --device "$DEVICE_ID" "$app" 1>&2
  elif command -v ios-deploy >/dev/null 2>&1; then
    ios-deploy --id "$DEVICE_ID" --bundle "$app" --no-wifi 1>&2
  else
    echo "error: need Xcode 15+ (devicectl) or ios-deploy for physical devices" >&2
    exit 1
  fi
}

install_android_artifact() {
  local apk="$1"
  if [[ -z "$apk" || ! -f "$apk" ]]; then
    echo "error: no APK to install (build may have been skipped — re-run without install-only)" >&2
    exit 1
  fi
  adb -s "$DEVICE_ID" install -r "$apk" 1>&2
}

ensure_deps() {
  [[ -d node_modules ]] || { echo "installing JS dependencies..." >&2; $PM_INSTALL 1>&2; }
}

# ── local build ─────────────────────────────────────────────────────
build_local() {
  case "$PLATFORM" in
    ios)
      local products
      if xcrun simctl list devices --json 2>/dev/null | grep -q "\"$DEVICE_ID\""; then
        products="$DERIVED_DATA/Build/Products/Release-iphonesimulator"
      else
        products="$DERIVED_DATA/Build/Products/Release-iphoneos"
      fi
      if [[ "$MODE" != "install-only" ]]; then
        ensure_deps
        [[ -d ios ]] || { echo "expo prebuild (ios)..." >&2; $PM_RUNNER prebuild --platform ios 1>&2; }
        local proj
        local ws
        ws=$(find ios -maxdepth 1 -name "*.xcworkspace" -print -quit 2>/dev/null || true)
        if [[ -n "$ws" ]]; then
          proj=(-workspace "$ws")
        else
          proj=(-project "$(find ios -maxdepth 1 -name '*.xcodeproj' -print -quit)")
        fi
        # Expo derives the scheme from the app name (unpredictable munging), so
        # if IOS_SCHEME is empty, discover it. List schemes from the app's
        # .xcodeproj — NOT the workspace, whose schemes are dominated by
        # CocoaPods (building one of those succeeds but produces no app .app).
        local scheme="$IOS_SCHEME"
        if [[ -z "$scheme" ]]; then
          local appproj
          appproj=$(find ios -maxdepth 1 -name '*.xcodeproj' -print -quit)
          scheme=$(xcodebuild -list -project "$appproj" 2>/dev/null \
            | awk '/Schemes:/{f=1; next} f && NF {print $1; exit}')
        fi
        if [[ -z "$scheme" ]]; then
          echo "error: could not determine an iOS scheme; set IOS_SCHEME in the script" >&2
          exit 1
        fi
        local dest
        if xcrun simctl list devices --json 2>/dev/null | grep -q "\"$DEVICE_ID\""; then
          dest="platform=iOS Simulator,id=$DEVICE_ID"
        else
          dest="platform=iOS,id=$DEVICE_ID"
        fi
        echo "building $scheme (Release) for $DEVICE_ID..." >&2
        xcodebuild "${proj[@]}" \
          -scheme "$scheme" \
          -configuration Release \
          -destination "$dest" \
          -derivedDataPath "$DERIVED_DATA" \
          build 1>&2
      else
        echo "install-only: reusing prior iOS build for $DEVICE_ID" >&2
      fi
      install_ios_artifact "$(find "$products" -maxdepth 1 -name '*.app' -type d -print -quit 2>/dev/null || true)"
      ;;
    android)
      if [[ "$MODE" != "install-only" ]]; then
        ensure_deps
        [[ -d android ]] || { echo "expo prebuild (android)..." >&2; $PM_RUNNER prebuild --platform android 1>&2; }
        echo "building Android (release)..." >&2
        ( cd android && ./gradlew :app:assembleRelease ) 1>&2
      else
        echo "install-only: reusing prior APK for $DEVICE_ID" >&2
      fi
      install_android_artifact "$(find android/app/build/outputs/apk/release -name '*.apk' -print -quit 2>/dev/null || true)"
      ;;
    *)
      echo "error: unknown platform $PLATFORM" >&2
      exit 1
      ;;
  esac
}

# ── EAS cloud build ─────────────────────────────────────────────────
# NOTE: This path requires an Expo account (EXPO_TOKEN) and hits Expo's
# servers. It is written but UNVERIFIED in golem's own test suite (no
# account in CI). Validate against a real Expo project before relying on it.
build_eas() {
  if ! command -v eas >/dev/null 2>&1; then
    echo "error: eas-cli not found — install it (npm i -g eas-cli) for EXPO_BUILD_MODE=eas" >&2
    exit 1
  fi
  if [[ -z "${EXPO_TOKEN:-}" ]]; then
    echo "error: EXPO_BUILD_MODE=eas requires EXPO_TOKEN (non-interactive auth). Set it via --var + install_env or the environment." >&2
    exit 1
  fi

  local eas_platform="$PLATFORM"   # eas uses ios|android, same as golem
  local artifact_url=""

  if [[ "$MODE" != "install-only" ]]; then
    # Reuse the latest finished build unless --rebuild forces a fresh one.
    if [[ "$GOLEM_REBUILD" != "1" ]]; then
      echo "eas: looking for a finished $eas_platform build on profile '$EAS_PROFILE'..." >&2
      artifact_url=$(eas build:list --platform "$eas_platform" --profile "$EAS_PROFILE" \
        --status finished --limit 1 --json --non-interactive 2>/dev/null \
        | grep -o '"artifacts":[^}]*"applicationArchiveUrl":"[^"]*"' \
        | grep -o 'https://[^"]*' | head -1 || true)
    fi
    if [[ -z "$artifact_url" ]]; then
      echo "eas: no reusable build (or --rebuild) — starting a cloud build..." >&2
      eas build --platform "$eas_platform" --profile "$EAS_PROFILE" --non-interactive 1>&2
      artifact_url=$(eas build:list --platform "$eas_platform" --profile "$EAS_PROFILE" \
        --status finished --limit 1 --json --non-interactive 2>/dev/null \
        | grep -o '"artifacts":[^}]*"applicationArchiveUrl":"[^"]*"' \
        | grep -o 'https://[^"]*' | head -1 || true)
    fi
    if [[ -z "$artifact_url" ]]; then
      echo "error: eas build produced no downloadable artifact" >&2
      exit 1
    fi
    mkdir -p build/eas
    echo "eas: downloading $artifact_url" >&2
    curl -fSL "$artifact_url" -o "build/eas/app-$eas_platform.bin" 1>&2
  else
    echo "install-only: reusing prior EAS artifact for $DEVICE_ID" >&2
  fi

  case "$PLATFORM" in
    ios)
      # EAS simulator builds ship a .tar.gz of the .app; device builds an .ipa.
      rm -rf build/eas/ios-extract && mkdir -p build/eas/ios-extract
      tar -xzf "build/eas/app-ios.bin" -C build/eas/ios-extract 2>/dev/null || true
      local app
      app=$(find build/eas/ios-extract -maxdepth 3 -name '*.app' -type d -print -quit 2>/dev/null || true)
      if [[ -z "$app" ]]; then
        # Not a simulator tarball — assume .ipa for a physical device.
        install_ios_artifact "$(find build/eas -maxdepth 1 -name '*.bin' -print -quit)"
      else
        install_ios_artifact "$app"
      fi
      ;;
    android)
      cp -f build/eas/app-android.bin build/eas/app-android.apk
      install_android_artifact build/eas/app-android.apk
      ;;
    *)
      echo "error: unknown platform $PLATFORM" >&2
      exit 1
      ;;
  esac
}

case "$BUILD_MODE" in
  local) build_local ;;
  eas)   build_eas ;;
  *)
    echo "error: unknown EXPO_BUILD_MODE='$BUILD_MODE' (expected 'local' or 'eas')" >&2
    exit 1
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
