#!/usr/bin/env bash
# golem install script — native iOS (simulator + physical)
#
# Invoked by golem before each flow to build and install the app onto a
# target device. Runs from the project root.
#
# Args:
#   $1 = platform (always "ios" for this template)
#   $2 = device UDID
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default)
#
# Detects simulator vs physical device by checking simctl. Physical device
# install requires Xcode 15+ (`xcrun devicectl`).
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_UDID="${2:?device UDID required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
XCODE_PROJECT="test-app-b/ios/GolemTestB.xcodeproj"       # e.g. MyApp.xcodeproj or MyApp.xcworkspace
XCODE_SCHEME="GolemTestB"         # Xcode scheme name
CONFIGURATION="Debug"        # Debug or Release
DERIVED_DATA="${DERIVED_DATA:-./build/DerivedData}"

# Determine project flag
PROJECT_FLAG=()
if [[ "$XCODE_PROJECT" == *.xcworkspace ]]; then
  PROJECT_FLAG=(-workspace "$XCODE_PROJECT")
else
  PROJECT_FLAG=(-project "$XCODE_PROJECT")
fi

# Detect simulator vs physical device.
IS_SIMULATOR=0
if xcrun simctl list devices --json 2>/dev/null | grep -q "\"$DEVICE_UDID\""; then
  IS_SIMULATOR=1
fi

if [[ "$IS_SIMULATOR" == "1" ]]; then
  DEST="platform=iOS Simulator,id=$DEVICE_UDID"
  PRODUCTS_DIR="$DERIVED_DATA/Build/Products/$CONFIGURATION-iphonesimulator"
else
  DEST="platform=iOS,id=$DEVICE_UDID"
  PRODUCTS_DIR="$DERIVED_DATA/Build/Products/$CONFIGURATION-iphoneos"
fi

if [[ "$MODE" != "install-only" ]]; then
  echo "building $XCODE_SCHEME ($CONFIGURATION) for $DEVICE_UDID..." >&2

  xcodebuild \
    "${PROJECT_FLAG[@]}" \
    -scheme "$XCODE_SCHEME" \
    -configuration "$CONFIGURATION" \
    -destination "$DEST" \
    -derivedDataPath "$DERIVED_DATA" \
    build \
    -quiet 1>&2
else
  echo "install-only: reusing prior build for $DEVICE_UDID" >&2
fi

# Locate the .app bundle
APP_PATH=$(find "$PRODUCTS_DIR" -maxdepth 1 -name "*.app" -type d -print -quit)

if [[ -z "$APP_PATH" ]]; then
  echo "error: no .app bundle found in $PRODUCTS_DIR (build may have been skipped — re-run without install-only)" >&2
  exit 1
fi

echo "installing $APP_PATH on $DEVICE_UDID..." >&2

if [[ "$IS_SIMULATOR" == "1" ]]; then
  xcrun simctl install "$DEVICE_UDID" "$APP_PATH" 1>&2
else
  if xcrun devicectl --version >/dev/null 2>&1; then
    xcrun devicectl device install app --device "$DEVICE_UDID" "$APP_PATH" 1>&2
  elif command -v ios-deploy >/dev/null 2>&1; then
    ios-deploy --id "$DEVICE_UDID" --bundle "$APP_PATH" --no-wifi 1>&2
  else
    echo "error: need Xcode 15+ (devicectl) or ios-deploy to install on physical devices" >&2
    exit 1
  fi
fi

echo "installed $BUNDLE_ID on $DEVICE_UDID" >&2
