#!/usr/bin/env bash
# golem install script — Flutter
#
# Invoked by golem before each flow to build and install a Flutter app onto
# a target simulator/emulator or physical device. Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Builds with `flutter build apk` / `flutter build ios`, then installs the
# artifact that build wrote with adb (Android) or simctl / devicectl (iOS).
# Not `flutter install`: it uninstalls the app first, which wipes its data
# and granted permissions on every install.
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE        = "debug" (default) | "profile" | "release". A simulator
#                       runs debug only; a physical iOS device runs profile or
#                       release only (a debug build needs the debugger to start).
#   FLAVOR            = Flutter flavor (Gradle product flavor / Xcode scheme)
#   DART_DEFINES      = space-separated KEY=VALUE pairs, each passed as
#                       --dart-define (a value cannot contain a space)
#   DART_DEFINE_FILE  = .json or .env file for --dart-define-from-file
#   FLUTTER_BUILD_ARGS = extra `flutter build` flags, word-split
#   DEVELOPMENT_TEAM  = Apple team ID for a physical iOS device
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
FLUTTER_DIR="test-app-d"     # path to the Flutter project (contains pubspec.yaml)
FLUTTER_CMD="flutter"     # Flutter CLI: flutter | fvm flutter

BUILD_TYPE="${BUILD_TYPE:-debug}"
FLAVOR="${FLAVOR:-}"
DART_DEFINES="${DART_DEFINES:-}"
DART_DEFINE_FILE="${DART_DEFINE_FILE:-}"
FLUTTER_BUILD_ARGS="${FLUTTER_BUILD_ARGS:-}"
DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"

case "$BUILD_TYPE" in
  debug | profile | release) ;;
  *)
    echo "error: unknown BUILD_TYPE='$BUILD_TYPE' (expected 'debug', 'profile' or 'release')" >&2
    exit 1
    ;;
esac

BUILD_ARGS=("--$BUILD_TYPE")
if [[ -n "$FLAVOR" ]]; then BUILD_ARGS+=(--flavor "$FLAVOR"); fi
for define in $DART_DEFINES; do BUILD_ARGS+=("--dart-define=$define"); done
if [[ -n "$DART_DEFINE_FILE" ]]; then BUILD_ARGS+=("--dart-define-from-file=$DART_DEFINE_FILE"); fi
# Word-split on purpose: FLUTTER_BUILD_ARGS is a list of flags.
# shellcheck disable=SC2206
if [[ -n "$FLUTTER_BUILD_ARGS" ]]; then BUILD_ARGS+=($FLUTTER_BUILD_ARGS); fi

cd "$FLUTTER_DIR"

# ── shared helpers (spliced in by `golem install-script`) ───────────

# Seconds-since-epoch mtime of a path. BSD and GNU `stat` disagree on the
# flag, and a stale-artifact guard is useless if the call aborts — which is
# what a bare `stat -f %m` does everywhere that isn't macOS.
#
# Probed once into an array rather than tried-and-fallen-back per call:
# GNU `stat -f` is --file-system, so it can print something for the file
# before failing on the format operand, and `A || B` in a command
# substitution would capture both halves as one corrupt number.
if stat -c %Y . >/dev/null 2>&1; then
  GOLEM_STAT=(stat -c %Y)     # GNU coreutils
else
  GOLEM_STAT=(stat -f %m)     # BSD / macOS
fi
golem_mtime() {
  "${GOLEM_STAT[@]}" "$1"
}

# Fail when $1 was not written at or after $2 (seconds since epoch, taken
# before the build started). A build step that fails quietly leaves the
# previous artifact in place, and installing it would test old code.
golem_require_fresh() {
  local m
  m=$(golem_mtime "$1")
  if (( m < $2 )); then
    echo "error: $1 was not refreshed by this build (mtime $m < build start $2);" >&2
    echo "       refusing to install a stale artifact." >&2
    return 1
  fi
}

# True when $1 is the UDID of a simulator (not a physical device).
#
# Matched against the "udid" field only, never a bare quoted string anywhere
# in the JSON. Captured, not piped into `grep -q`: grep exits at the first
# match, a large device list then takes SIGPIPE, and under pipefail that
# reads as "not a simulator".
golem_is_simulator() {
  local json
  json=$(xcrun simctl list devices --json 2>/dev/null) || return 1
  grep -Eq "\"udid\"[[:space:]]*:[[:space:]]*\"$1\"" <<<"$json"
}

# Newest (by mtime) of the paths `find "$@"` prints; prints nothing when
# there is no match or the search root is missing.
#
# Not `find … -print -quit` (whichever match the filesystem lists first) and
# not `ls -t | head` (SIGPIPE under pipefail). A missing root must yield an
# empty result rather than a nonzero exit: as an assignment's command
# substitution it would kill the script under `set -e` before the caller's
# own "no artifact" error could name the problem.
golem_newest() {
  local best="" best_m=0 p m
  while IFS= read -r p; do
    m=$(golem_mtime "$p")
    if (( m > best_m )); then best="$p"; best_m="$m"; fi
  done < <(find "$@" 2>/dev/null || true)
  printf '%s' "$best"
}

# Newest installable APK under $1. $2 optionally narrows it to one variant
# directory as AGP lays them out (`release`, `free/debug`), so a flavor or
# buildType build is never mixed up with another one left behind.
# Test APKs and unsigned release APKs are never installable, so never picked.
golem_pick_apk() {
  local dir="$1" variant="${2:-}"
  local filter=(-name '*.apk' ! -name '*-androidTest.apk' ! -name '*-unsigned.apk')
  if [[ -n "$variant" ]]; then filter+=(-path "*/$variant/*"); fi
  golem_newest "$dir" -type f "${filter[@]}"
}

# Report on stderr why `golem_pick_apk "$1"` found nothing. An unsigned
# release APK is the common case, and "no APK found" would send the reader
# looking for a build that did run.
golem_no_apk_error() {
  if [[ -n "$(golem_newest "$1" -type f -name '*-unsigned.apk')" ]]; then
    echo "error: $1 holds only unsigned APKs, and Android installs only signed ones." >&2
    echo "       Add a release signingConfig to the Gradle project, or build debug." >&2
  else
    echo "error: no APK found under $1 (build may have been skipped — re-run without install-only)" >&2
  fi
}

# Newest .app bundle at most $2 (default 1) levels under $1. Never one
# nested inside another bundle (an App Clip or watch app): those are not
# what gets installed, and their mtimes are not ordered against the outer one.
golem_pick_app() {
  golem_newest "$1" -maxdepth "${2:-1}" -name '*.app' -type d -prune
}

# ── iOS build + install (spliced in by `golem install-script`) ──────
# Needs the helpers partial above it.

# Install the .app at $2 onto the simulator or physical device $1.
golem_ios_install_app() {
  local udid="$1" app="$2"
  echo "installing $app on $udid..." >&2
  if golem_is_simulator "$udid"; then
    xcrun simctl install "$udid" "$app" 1>&2 || return 1
  elif xcrun devicectl --version >/dev/null 2>&1; then
    xcrun devicectl device install app --device "$udid" "$app" 1>&2 || return 1
  elif command -v ios-deploy >/dev/null 2>&1; then
    ios-deploy --id "$udid" --bundle "$app" --no-wifi 1>&2 || return 1
  else
    echo "error: need Xcode 15+ (devicectl) or ios-deploy to install on physical devices" >&2
    return 1
  fi
}

# Build an Xcode scheme for the target device and print the path of the
# .app that build wrote, on stdout. Everything else goes to stderr.
#
#   golem_ios_build <project> <scheme> <configuration> <xcconfig> \
#                   <development_team> <derived_data> <udid> <mode>
#
# <project> is an .xcodeproj or .xcworkspace. <xcconfig> and
# <development_team> may be empty; the team applies to a physical device
# only. <mode> is "install-only" to skip the build and reuse the previous
# .app. Every failure returns 1 explicitly: a caller in an `if` or `||`
# turns `set -e` off for the whole function body.
golem_ios_build() {
  local project="$1" scheme="$2" configuration="$3" xcconfig="$4"
  local team="$5" derived_data="$6" udid="$7" mode="$8"
  local is_simulator=0 products_dir build_start app
  local build_args=()

  if [[ "$project" == *.xcworkspace ]]; then
    build_args+=(-workspace "$project")
  else
    build_args+=(-project "$project")
  fi
  build_args+=(-scheme "$scheme" -configuration "$configuration" -derivedDataPath "$derived_data")
  if [[ -n "$xcconfig" ]]; then build_args+=(-xcconfig "$xcconfig"); fi

  if golem_is_simulator "$udid"; then is_simulator=1; fi
  if [[ "$is_simulator" == "1" ]]; then
    build_args+=(-destination "platform=iOS Simulator,id=$udid")
    products_dir="$derived_data/Build/Products/$configuration-iphonesimulator"
  else
    build_args+=(-destination "platform=iOS,id=$udid")
    products_dir="$derived_data/Build/Products/$configuration-iphoneos"
    if [[ -n "$team" ]]; then
      build_args+=(-allowProvisioningUpdates "DEVELOPMENT_TEAM=$team")
    fi
  fi

  if [[ "$mode" != "install-only" ]]; then
    build_start=$(date +%s)
    echo "building $scheme ($configuration) for $udid..." >&2
    # An incremental build with nothing to do leaves the .app unwritten, which
    # the freshness guard cannot tell from a stale one. Removing the bundles
    # makes every build write its .app, at the cost of re-running the link
    # and copy steps only.
    rm -rf "$products_dir"/*.app

    # No `-quiet`: it hides the compiler and signing errors a failed build
    # needs to show.
    if ! xcodebuild "${build_args[@]}" build 1>&2; then
      if [[ "$is_simulator" == "0" ]]; then
        echo "error: xcodebuild failed for physical device $udid. If the errors above" >&2
        echo "       are about signing or provisioning, set DEVELOPMENT_TEAM to your Apple" >&2
        echo "       team ID (install_env or the shell). Then the build signs with" >&2
        echo "       -allowProvisioningUpdates." >&2
      fi
      return 1
    fi
  else
    echo "install-only: reusing prior build for $udid" >&2
  fi

  app=$(golem_pick_app "$products_dir")
  if [[ -z "$app" ]]; then
    echo "error: no .app bundle found in $products_dir (build may have been skipped — re-run without install-only)" >&2
    return 1
  fi
  if [[ "$mode" != "install-only" ]]; then
    golem_require_fresh "$app" "$build_start" || return 1
  fi
  printf '%s\n' "$app"
}

# golem_ios_build, then install the .app it wrote. Same arguments.
golem_ios_build_install() {
  local app
  app=$(golem_ios_build "$@") || return 1
  golem_ios_install_app "$7" "$app"
}

case "$PLATFORM" in
  android)
    # The Gradle output, laid out by variant (`debug`, `free/release`). The
    # `flutter-apk/` copy has no variant directories to narrow a pick by.
    APK_ROOT="build/app/outputs/apk"
    VARIANT="$BUILD_TYPE"
    if [[ -n "$FLAVOR" ]]; then VARIANT="$FLAVOR/$BUILD_TYPE"; fi
    if [[ "$MODE" != "install-only" ]]; then
      BUILD_START_TS=$(date +%s)
      # An up-to-date build leaves the APK unwritten, which the freshness
      # guard cannot tell from a stale one. With the APK gone, Gradle
      # repackages it.
      rm -rf "${APK_ROOT:?}/$VARIANT"
      echo "flutter build apk ($VARIANT)..." >&2
      $FLUTTER_CMD build apk "${BUILD_ARGS[@]}" 1>&2
    else
      echo "install-only: reusing prior APK for $DEVICE_ID" >&2
    fi
    APK=$(golem_pick_apk "$APK_ROOT" "$VARIANT")
    if [[ -z "$APK" ]]; then
      golem_no_apk_error "$APK_ROOT"
      exit 1
    fi
    if [[ "$MODE" != "install-only" ]]; then
      golem_require_fresh "$APK" "$BUILD_START_TS" || exit 1
    fi
    echo "installing $APK on $DEVICE_ID..." >&2
    adb -s "$DEVICE_ID" install -r "$APK" 1>&2
    ;;
  ios)
    if golem_is_simulator "$DEVICE_ID"; then
      if [[ "$BUILD_TYPE" != debug ]]; then
        echo "error: Flutter builds only debug for a simulator (BUILD_TYPE='$BUILD_TYPE')" >&2
        exit 1
      fi
      PRODUCTS_DIR="build/ios/iphonesimulator"
      BUILD_ARGS+=(--simulator)
    else
      if [[ "$BUILD_TYPE" == debug ]]; then
        echo "error: a debug Flutter build starts on a physical iOS device only under the debugger;" >&2
        echo "       set BUILD_TYPE=profile or BUILD_TYPE=release" >&2
        exit 1
      fi
      PRODUCTS_DIR="build/ios/iphoneos"
      # Flutter passes FLUTTER_XCODE_<setting> to xcodebuild as <setting>.
      if [[ -n "$DEVELOPMENT_TEAM" ]]; then export FLUTTER_XCODE_DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM"; fi
    fi
    if [[ "$MODE" != "install-only" ]]; then
      BUILD_START_TS=$(date +%s)
      rm -rf "$PRODUCTS_DIR"/*.app
      echo "flutter build ios ($BUILD_TYPE, $PRODUCTS_DIR)..." >&2
      $FLUTTER_CMD build ios "${BUILD_ARGS[@]}" 1>&2
    else
      echo "install-only: reusing prior build for $DEVICE_ID" >&2
    fi
    APP_PATH=$(golem_pick_app "$PRODUCTS_DIR")
    if [[ -z "$APP_PATH" ]]; then
      echo "error: no .app under $PRODUCTS_DIR (build may have been skipped — re-run without install-only)" >&2
      exit 1
    fi
    if [[ "$MODE" != "install-only" ]]; then
      golem_require_fresh "$APP_PATH" "$BUILD_START_TS" || exit 1
    fi
    golem_ios_install_app "$DEVICE_ID" "$APP_PATH"
    ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
