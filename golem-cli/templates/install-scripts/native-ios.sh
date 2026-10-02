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
#        or empty for full build+install (default).
#        Golem currently always passes empty; the flag is supported for manual
#        dev-iteration and for a future golem-side build-once optimisation
#        (see roadmap: "Install Cache: Build-Once, Install-to-Many").
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   CONFIGURATION    = overrides the build configuration below (Debug, Release)
#   XCODE_SCHEME     = overrides the scheme below
#   XCCONFIG         = path to an .xcconfig, passed as `xcodebuild -xcconfig`
#   DEVELOPMENT_TEAM = Apple team ID for a physical device. Passed as the
#                      DEVELOPMENT_TEAM build setting, with
#                      -allowProvisioningUpdates. Unset: the project's own
#                      signing settings apply.
#   DERIVED_DATA     = xcodebuild derived-data dir (default ./build/DerivedData)
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
XCODE_PROJECT="{{XCODE_PROJECT}}"       # e.g. MyApp.xcodeproj or MyApp.xcworkspace
XCODE_SCHEME="${XCODE_SCHEME:-{{XCODE_SCHEME}}}"         # Xcode scheme name
CONFIGURATION="${CONFIGURATION:-{{CONFIGURATION}}}"        # Debug or Release
XCCONFIG="${XCCONFIG:-}"
DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"
DERIVED_DATA="${DERIVED_DATA:-./build/DerivedData}"

{{>helpers}}

# Determine project flag
PROJECT_FLAG=()
if [[ "$XCODE_PROJECT" == *.xcworkspace ]]; then
  PROJECT_FLAG=(-workspace "$XCODE_PROJECT")
else
  PROJECT_FLAG=(-project "$XCODE_PROJECT")
fi

IS_SIMULATOR=0
if golem_is_simulator "$DEVICE_UDID"; then
  IS_SIMULATOR=1
fi

BUILD_ARGS=(
  "${PROJECT_FLAG[@]}"
  -scheme "$XCODE_SCHEME"
  -configuration "$CONFIGURATION"
  -derivedDataPath "$DERIVED_DATA"
)
if [[ -n "$XCCONFIG" ]]; then BUILD_ARGS+=(-xcconfig "$XCCONFIG"); fi

if [[ "$IS_SIMULATOR" == "1" ]]; then
  BUILD_ARGS+=(-destination "platform=iOS Simulator,id=$DEVICE_UDID")
  PRODUCTS_DIR="$DERIVED_DATA/Build/Products/$CONFIGURATION-iphonesimulator"
else
  BUILD_ARGS+=(-destination "platform=iOS,id=$DEVICE_UDID")
  PRODUCTS_DIR="$DERIVED_DATA/Build/Products/$CONFIGURATION-iphoneos"
  if [[ -n "$DEVELOPMENT_TEAM" ]]; then
    BUILD_ARGS+=(-allowProvisioningUpdates "DEVELOPMENT_TEAM=$DEVELOPMENT_TEAM")
  fi
fi

if [[ "$MODE" != "install-only" ]]; then
  BUILD_START_TS=$(date +%s)
  echo "building $XCODE_SCHEME ($CONFIGURATION) for $DEVICE_UDID..." >&2
  # An incremental build with nothing to do leaves the .app unwritten, which
  # the freshness guard cannot tell from a stale one. Removing the bundles
  # makes every build write its .app, at the cost of re-running the link
  # and copy steps only.
  rm -rf "$PRODUCTS_DIR"/*.app

  # No `-quiet`: it hides the compiler and signing errors a failed build
  # needs to show.
  if ! xcodebuild "${BUILD_ARGS[@]}" build 1>&2; then
    if [[ "$IS_SIMULATOR" == "0" ]]; then
      echo "error: xcodebuild failed for physical device $DEVICE_UDID. If the errors above" >&2
      echo "       are about signing or provisioning, set DEVELOPMENT_TEAM to your Apple" >&2
      echo "       team ID (install_env or the shell). Then the build signs with" >&2
      echo "       -allowProvisioningUpdates." >&2
    fi
    exit 1
  fi
else
  echo "install-only: reusing prior build for $DEVICE_UDID" >&2
fi

APP_PATH=$(golem_pick_app "$PRODUCTS_DIR")

if [[ -z "$APP_PATH" ]]; then
  echo "error: no .app bundle found in $PRODUCTS_DIR (build may have been skipped — re-run without install-only)" >&2
  exit 1
fi
if [[ "$MODE" != "install-only" ]]; then
  golem_require_fresh "$APP_PATH" "$BUILD_START_TS" || exit 1
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
