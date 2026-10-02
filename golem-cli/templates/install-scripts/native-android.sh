#!/usr/bin/env bash
# golem install script — native Android
#
# Invoked by golem before each flow to build and install the app onto a
# target emulator/device. Runs from the project root.
#
# Args:
#   $1 = platform (always "android" for this template)
#   $2 = device serial (adb -s)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous APK,
#        or empty for full build+install (default).
#        Golem currently always passes empty; the flag is supported for manual
#        dev-iteration and for a future golem-side build-once optimisation
#        (see roadmap: "Install Cache: Build-Once, Install-to-Many").
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE = Gradle build type (default "debug"). A release build needs a
#                signingConfig; Android installs only signed APKs.
#   FLAVOR     = Gradle product flavor, the full variant flavor name for
#                several dimensions (for example "freeProd"). Default: none.
#
# Builds with `assemble<Flavor><BuildType>`, then installs the APK that build
# wrote with `adb install`.
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_SERIAL="${2:?device serial required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
GRADLE_ROOT="{{GRADLE_ROOT}}"           # directory containing settings.gradle (cd'd before gradle)
MODULE_NAME="{{MODULE_NAME}}"           # gradle submodule (e.g. app)

BUILD_TYPE="${BUILD_TYPE:-debug}"
FLAVOR="${FLAVOR:-}"

{{>helpers}}

{{>android}}

golem_android_build_install "$GRADLE_ROOT" "$MODULE_NAME" "$FLAVOR" "$BUILD_TYPE" \
  "$DEVICE_SERIAL" "$BUNDLE_ID" "$MODE"

echo "installed $BUNDLE_ID on $DEVICE_SERIAL" >&2
