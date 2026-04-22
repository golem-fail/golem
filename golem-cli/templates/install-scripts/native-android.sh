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
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_SERIAL="${2:?device serial required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
GRADLE_ROOT="{{GRADLE_ROOT}}"           # directory containing settings.gradle (cd'd before gradle)
MODULE_NAME="{{MODULE_NAME}}"           # gradle submodule (e.g. app)
GRADLE_TASK="{{GRADLE_TASK}}"           # e.g. installDebug, assembleDebug

if [[ "$MODE" == "install-only" ]]; then
  echo "install-only: reusing prior APK for $DEVICE_SERIAL" >&2
  # Find the APK produced by a prior gradle build (common Android layouts).
  APK=$(find "$GRADLE_ROOT/$MODULE_NAME/build/outputs/apk" -name "*.apk" -print -quit 2>/dev/null)
  if [[ -z "$APK" ]]; then
    echo "error: no APK found under $GRADLE_ROOT/$MODULE_NAME/build/outputs/apk (build may have been skipped — re-run without install-only)" >&2
    exit 1
  fi
  adb -s "$DEVICE_SERIAL" install -r "$APK" 1>&2
else
  echo "building :${MODULE_NAME}:${GRADLE_TASK} (${GRADLE_ROOT})..." >&2
  cd "$GRADLE_ROOT"
  # Let gradle install directly onto the specified device via ANDROID_SERIAL
  ANDROID_SERIAL="$DEVICE_SERIAL" ./gradlew ":${MODULE_NAME}:${GRADLE_TASK}" 1>&2
fi

echo "installed $BUNDLE_ID on $DEVICE_SERIAL" >&2
