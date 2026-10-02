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

# First letter upper-cased. Not `${1^}`: that needs bash 4, and macOS ships 3.2.
capitalize() {
  printf '%s%s' "$(printf '%s' "${1:0:1}" | tr '[:lower:]' '[:upper:]')" "${1:1}"
}

APK_ROOT="$GRADLE_ROOT/$MODULE_NAME/build/outputs/apk"
if [[ -n "$FLAVOR" ]]; then
  VARIANT_DIR="$FLAVOR/$BUILD_TYPE"
else
  VARIANT_DIR="$BUILD_TYPE"
fi
TASK=":${MODULE_NAME}:assemble$(capitalize "$FLAVOR")$(capitalize "$BUILD_TYPE")"

# Not an `install*` task: gradle would install inside the build, so the
# freshness guard and the bundle-id check below would never see the APK.
if [[ "$MODE" != "install-only" ]]; then
  BUILD_START_TS=$(date +%s)
  echo "building $TASK ($GRADLE_ROOT)..." >&2
  # Gradle leaves an up-to-date APK unwritten, which the freshness guard
  # cannot tell from a stale one. Removing this variant's outputs makes every
  # build write its APK, at the cost of re-running only the packaging.
  rm -rf "${APK_ROOT:?}/$VARIANT_DIR"
  ( cd "$GRADLE_ROOT" && ./gradlew "$TASK" ) 1>&2
else
  echo "install-only: reusing prior APK for $DEVICE_SERIAL" >&2
fi

APK=$(golem_pick_apk "$APK_ROOT" "$VARIANT_DIR")
if [[ -z "$APK" ]]; then
  golem_no_apk_error "$APK_ROOT"
  exit 1
fi
if [[ "$MODE" != "install-only" ]]; then
  golem_require_fresh "$APK" "$BUILD_START_TS" || exit 1
fi

# The APK's applicationId must be the bundle golem launches. A mismatch
# installs fine and then fails at launch with nothing pointing here. Skipped
# when no SDK build-tools are found: a missing check beats a false failure.
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
AAPT2=""
if [[ -n "$SDK" ]]; then AAPT2=$(golem_newest "$SDK/build-tools" -name aapt2 -type f); fi
if [[ -n "$AAPT2" ]]; then
  APK_ID=$("$AAPT2" dump packagename "$APK" 2>/dev/null || true)
  if [[ -n "$APK_ID" && "$APK_ID" != "$BUNDLE_ID" ]]; then
    echo "error: $APK has applicationId '$APK_ID', but golem launches '$BUNDLE_ID'." >&2
    echo "       Fix the bundle in golem.toml, or FLAVOR/BUILD_TYPE (an applicationIdSuffix?)." >&2
    exit 1
  fi
fi

echo "installing $APK on $DEVICE_SERIAL..." >&2
adb -s "$DEVICE_SERIAL" install -r "$APK" 1>&2

echo "installed $BUNDLE_ID on $DEVICE_SERIAL" >&2
