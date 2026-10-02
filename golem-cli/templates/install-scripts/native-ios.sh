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

{{>ios}}

golem_ios_build_install "$XCODE_PROJECT" "$XCODE_SCHEME" "$CONFIGURATION" "$XCCONFIG" \
  "$DEVELOPMENT_TEAM" "$DERIVED_DATA" "$DEVICE_UDID" "$MODE"

echo "installed $BUNDLE_ID on $DEVICE_UDID" >&2
