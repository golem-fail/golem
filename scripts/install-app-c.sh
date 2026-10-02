#!/usr/bin/env bash
# golem install script — Capacitor / Ionic
#
# Invoked by golem before each flow to build and install a Capacitor app onto
# a target simulator/emulator or physical device. Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Steps: install JS dependencies when they changed, build the web app, copy
# it into the native project with `cap sync`, then build and install with
# Gradle (Android) or xcodebuild (iOS). The native projects (android/, ios/)
# are source in a Capacitor app, so this script never creates them.
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE       = "debug" (default) | "release". Selects the Gradle build
#                      type and the Xcode configuration (Debug / Release).
#                      A release build needs signing on Android, and
#                      `webContentsDebuggingEnabled: true` in the Capacitor
#                      config on both platforms: golem reads the page through
#                      the web inspector, which release builds turn off.
#   FLAVOR           = Gradle product flavor (default: none)
#   XCCONFIG         = path to an .xcconfig, passed as `xcodebuild -xcconfig`
#   DEVELOPMENT_TEAM = Apple team ID, for a physical iOS device
#   DERIVED_DATA     = xcodebuild derived-data dir (default ./build/DerivedData)
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
CAP_DIR="test-app-c"             # path to the Capacitor project (contains capacitor.config.*)
CAP_CMD="npx cap"             # Capacitor CLI runner: npx cap | yarn cap | pnpm cap | bunx cap
PM_INSTALL="npm install"       # dependency install: npm install | yarn | pnpm install | bun install
WEB_BUILD="npm run build"         # web build command (e.g. npm run build); empty = no build step
WEB_DIR="www"             # `webDir` from the Capacitor config

BUILD_TYPE="${BUILD_TYPE:-debug}"
FLAVOR="${FLAVOR:-}"
XCCONFIG="${XCCONFIG:-}"
DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"
DERIVED_DATA="${DERIVED_DATA:-./build/DerivedData}"

case "$BUILD_TYPE" in
  debug)   XCODE_CONFIGURATION=Debug ;;
  release) XCODE_CONFIGURATION=Release ;;
  *)
    echo "error: unknown BUILD_TYPE='$BUILD_TYPE' (expected 'debug' or 'release')" >&2
    exit 1
    ;;
esac

cd "$CAP_DIR"

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

# ── JS dependency freshness (spliced in by `golem install-script`) ──
# A gate that asks "does node_modules exist?" answers yes forever: after a
# lockfile change the build is redone against the PREVIOUS dependency tree,
# and the run reports green. golem's install cache can't catch this — it
# correctly reports a rebuild, and the thing being rebuilt is stale.
#
# So each generated tree records what it was generated FROM, and the gate
# compares that instead of merely checking for existence.
#
# Stamps live under node_modules: it is already ignored by every project's
# VCS, so nothing appears in `git status`, and a wipe (`npm ci`, `rm -rf
# node_modules`) takes the stamps with it.
GOLEM_STAMP_DIR="node_modules/.golem"

# Inputs that decide whether the installed dependency tree is current. Every
# lockfile flavour is listed rather than just this project's, so the stamp
# stays correct if the package manager is switched.
GOLEM_DEP_INPUTS=(package.json package-lock.json yarn.lock pnpm-lock.yaml bun.lockb bun.lock)

# Hash of the named files, in order. A file's NAME is hashed alongside its
# contents so that swapping one lockfile flavour for an identical-looking
# other still counts as a change. Missing files contribute nothing.
golem_hash() {
  local f
  for f in "$@"; do
    if [[ -f "$f" ]]; then printf '%s\n' "$f"; cat "$f"; fi
  done | shasum | cut -d' ' -f1
}

# True when stamp $1 is absent or records something other than $2.
golem_stale() {
  local stamp="$GOLEM_STAMP_DIR/$1"
  [[ -f "$stamp" ]] || return 0
  [[ "$(cat "$stamp" 2>/dev/null)" != "$2" ]]
}

# Record $2 as stamp $1. Every caller guards the preceding command with
# `|| return 1` rather than leaning on `set -e`: a stamp written after a
# failed install would remember the failure as done and skip the retry, and
# these templates are meant to be edited after scaffolding.
golem_stamp() {
  mkdir -p "$GOLEM_STAMP_DIR"
  printf '%s' "$2" > "$GOLEM_STAMP_DIR/$1"
}

# Install JS dependencies with the command $1 (`npm install`, `yarn`, …)
# when the inputs have moved since the last install. Does nothing when $1 is
# empty or there is no package.json: a project with no JS has nothing to
# install, and guessing would be worse than doing nothing.
golem_ensure_deps() {
  local pm_install="$1" want
  [[ -n "$pm_install" ]] || return 0
  [[ -f package.json ]] || return 0
  want=$(golem_hash "${GOLEM_DEP_INPUTS[@]}")
  if [[ -d node_modules ]] && ! golem_stale deps "$want"; then
    return 0
  fi
  echo "installing JS dependencies (dependency inputs changed)..." >&2
  $pm_install 1>&2 || return 1
  # Re-hash AFTER the install: package managers rewrite the lockfile as
  # part of installing, so stamping the pre-install hash would leave the
  # stamp stale the moment it was written and reinstall on every run.
  golem_stamp deps "$(golem_hash "${GOLEM_DEP_INPUTS[@]}")"
}

# ── Android build + install (spliced in by `golem install-script`) ──
# Needs the helpers partial above it.

# First letter upper-cased. Not `${1^}`: that needs bash 4, and macOS ships 3.2.
golem_capitalize() {
  printf '%s%s' "$(printf '%s' "${1:0:1}" | tr '[:lower:]' '[:upper:]')" "${1:1}"
}

# Build a Gradle module with `assemble<Flavor><BuildType>`, then install the
# APK that build wrote with `adb install`.
#
#   golem_android_build_install <gradle_root> <module> <flavor> <build_type> \
#                               <serial> <bundle_id> <mode>
#
# <flavor> may be empty. <mode> is "install-only" to skip the build and
# reuse the previous APK. Every failure returns 1 explicitly: a caller in an
# `if` or `||` turns `set -e` off for the whole function body.
golem_android_build_install() {
  local gradle_root="$1" module="$2" flavor="$3" build_type="$4"
  local serial="$5" bundle_id="$6" mode="$7"
  local apk_root="$gradle_root/$module/build/outputs/apk"
  local variant_dir task build_start apk
  if [[ -n "$flavor" ]]; then
    variant_dir="$flavor/$build_type"
  else
    variant_dir="$build_type"
  fi
  task=":${module}:assemble$(golem_capitalize "$flavor")$(golem_capitalize "$build_type")"

  # Not an `install*` task: gradle would install inside the build, so the
  # freshness guard and the bundle-id check below would never see the APK.
  if [[ "$mode" != "install-only" ]]; then
    build_start=$(date +%s)
    echo "building $task ($gradle_root)..." >&2
    # Gradle leaves an up-to-date APK unwritten, which the freshness guard
    # cannot tell from a stale one. Removing this variant's outputs makes every
    # build write its APK, at the cost of re-running only the packaging.
    rm -rf "${apk_root:?}/$variant_dir"
    ( cd "$gradle_root" && ./gradlew "$task" ) 1>&2 || return 1
  else
    echo "install-only: reusing prior APK for $serial" >&2
  fi

  apk=$(golem_pick_apk "$apk_root" "$variant_dir")
  if [[ -z "$apk" ]]; then
    golem_no_apk_error "$apk_root"
    return 1
  fi
  if [[ "$mode" != "install-only" ]]; then
    golem_require_fresh "$apk" "$build_start" || return 1
  fi

  # The APK's applicationId must be the bundle golem launches. A mismatch
  # installs fine and then fails at launch with nothing pointing here. Skipped
  # when no SDK build-tools are found: a missing check beats a false failure.
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}" aapt2="" apk_id
  if [[ -n "$sdk" ]]; then aapt2=$(golem_newest "$sdk/build-tools" -name aapt2 -type f); fi
  if [[ -n "$aapt2" ]]; then
    apk_id=$("$aapt2" dump packagename "$apk" 2>/dev/null || true)
    if [[ -n "$apk_id" && "$apk_id" != "$bundle_id" ]]; then
      echo "error: $apk has applicationId '$apk_id', but golem launches '$bundle_id'." >&2
      echo "       Fix the bundle in golem.toml, or FLAVOR/BUILD_TYPE (an applicationIdSuffix?)." >&2
      return 1
    fi
  fi

  echo "installing $apk on $serial..." >&2
  adb -s "$serial" install -r "$apk" 1>&2 || return 1
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

# Build an Xcode scheme for the target device, then install the .app that
# build wrote.
#
#   golem_ios_build_install <project> <scheme> <configuration> <xcconfig> \
#                           <development_team> <derived_data> <udid> <mode>
#
# <project> is an .xcodeproj or .xcworkspace. <xcconfig> and
# <development_team> may be empty; the team applies to a physical device
# only. <mode> is "install-only" to skip the build and reuse the previous
# .app. Every failure returns 1 explicitly: a caller in an `if` or `||`
# turns `set -e` off for the whole function body.
golem_ios_build_install() {
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

  golem_ios_install_app "$udid" "$app"
}

case "$PLATFORM" in
  ios|android) ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

if [[ ! -d "$PLATFORM" ]]; then
  echo "error: no $PLATFORM/ project in $CAP_DIR. Create it once with" >&2
  echo "       '$CAP_CMD add $PLATFORM' and commit it: it is source in a Capacitor app." >&2
  exit 1
fi

if [[ "$MODE" != "install-only" ]]; then
  golem_ensure_deps "$PM_INSTALL"

  if [[ -n "$WEB_BUILD" ]]; then
    WEB_BUILD_START=$(date +%s)
    echo "building the web app ($WEB_BUILD)..." >&2
    $WEB_BUILD 1>&2
    # `cap sync` copies whatever is in webDir. A web build that quietly did
    # nothing would ship the previous bundle inside a fresh native build.
    WEB_NEWEST=$(golem_newest "$WEB_DIR" -type f)
    if [[ -z "$WEB_NEWEST" ]]; then
      echo "error: $WEB_DIR is empty after '$WEB_BUILD'. Is WEB_DIR the webDir of the Capacitor config?" >&2
      exit 1
    fi
    golem_require_fresh "$WEB_NEWEST" "$WEB_BUILD_START"
  fi

  echo "cap sync $PLATFORM..." >&2
  $CAP_CMD sync "$PLATFORM" 1>&2
fi

case "$PLATFORM" in
  android)
    golem_android_build_install android app "$FLAVOR" "$BUILD_TYPE" \
      "$DEVICE_ID" "$BUNDLE_ID" "$MODE"
    ;;
  ios)
    # CocoaPods projects build through the workspace; Swift Package Manager
    # projects (the Capacitor 8 default) have only the .xcodeproj.
    XCODE_PROJECT="ios/App/App.xcodeproj"
    if [[ -d ios/App/App.xcworkspace ]]; then XCODE_PROJECT="ios/App/App.xcworkspace"; fi
    golem_ios_build_install "$XCODE_PROJECT" App "$XCODE_CONFIGURATION" "$XCCONFIG" \
      "$DEVELOPMENT_TEAM" "$DERIVED_DATA" "$DEVICE_ID" "$MODE"
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
