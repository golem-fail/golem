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

# ── freshness stamps ────────────────────────────────────────────────
# A gate that asks "does this directory exist?" answers yes forever: after a
# lockfile change the native build is redone against the PREVIOUS dependency
# tree, and the run reports green. golem's install cache can't catch this —
# it correctly reports a rebuild, and the thing being rebuilt is stale.
#
# So each generated tree records what it was generated FROM, and the gate
# compares that instead of merely checking for existence.
#
# Stamps live under node_modules: it is already ignored by every project's
# VCS, so nothing appears in `git status`, and a wipe (`npm ci`, `rm -rf
# node_modules`) takes the prebuild stamps with it — which conservatively
# re-runs prebuild after a dependency wipe rather than trusting a native
# project generated from a tree that is now gone.
GOLEM_STAMP_DIR="node_modules/.golem"

# Inputs that decide whether the installed dependency tree is current. Every
# lockfile flavour is listed rather than just this project's, so the stamp
# stays correct if the package manager is switched.
GOLEM_DEP_INPUTS=(package.json package-lock.json yarn.lock pnpm-lock.yaml bun.lockb bun.lock)
# Prebuild additionally depends on the Expo config, which is what decides the
# shape of the generated native project.
GOLEM_PREBUILD_INPUTS=("${GOLEM_DEP_INPUTS[@]}" app.json app.config.js app.config.ts)

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
# this template is meant to be edited after scaffolding.
golem_stamp() {
  mkdir -p "$GOLEM_STAMP_DIR"
  printf '%s' "$2" > "$GOLEM_STAMP_DIR/$1"
}

# ── shared install helpers ──────────────────────────────────────────
install_ios_artifact() {
  local app="$1"
  if [[ -z "$app" || ! -d "$app" ]]; then
    echo "error: no .app to install (build may have been skipped — re-run without install-only)" >&2
    exit 1
  fi
  if golem_is_simulator "$DEVICE_ID"; then
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
  local want
  want=$(golem_hash "${GOLEM_DEP_INPUTS[@]}")
  if [[ ! -d node_modules ]] || golem_stale deps "$want"; then
    echo "installing JS dependencies (dependency inputs changed)..." >&2
    $PM_INSTALL 1>&2 || return 1
    # Re-hash AFTER the install: package managers rewrite the lockfile as
    # part of installing, so stamping the pre-install hash would leave the
    # stamp stale the moment it was written and reinstall on every run.
    golem_stamp deps "$(golem_hash "${GOLEM_DEP_INPUTS[@]}")"
  fi
}

# Generate the native project for $1 (ios|android) when it is missing or was
# generated from different inputs. Plain `prebuild`, never `--clean`: a
# downstream project may have hand-edited its native directory, and silently
# discarding that would be worse than the staleness this is fixing. If the
# regenerated project needs a clean slate, delete the directory.
ensure_prebuild() {
  local platform="$1"
  local want
  want=$(golem_hash "${GOLEM_PREBUILD_INPUTS[@]}")
  if [[ ! -d "$platform" ]] || golem_stale "prebuild-$platform" "$want"; then
    echo "expo prebuild ($platform)..." >&2
    $PM_RUNNER prebuild --platform "$platform" 1>&2 || return 1
    # Re-hash after, for the same reason: prebuild may touch the config it
    # was generated from.
    golem_stamp "prebuild-$platform" "$(golem_hash "${GOLEM_PREBUILD_INPUTS[@]}")"
  fi
}

# ── local build ─────────────────────────────────────────────────────
build_local() {
  case "$PLATFORM" in
    ios)
      local products
      if golem_is_simulator "$DEVICE_ID"; then
        products="$DERIVED_DATA/Build/Products/Release-iphonesimulator"
      else
        products="$DERIVED_DATA/Build/Products/Release-iphoneos"
      fi
      if [[ "$MODE" != "install-only" ]]; then
        ensure_deps
        ensure_prebuild ios
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
        if golem_is_simulator "$DEVICE_ID"; then
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
      install_ios_artifact "$(golem_pick_app "$products")"
      ;;
    android)
      if [[ "$MODE" != "install-only" ]]; then
        ensure_deps
        ensure_prebuild android
        echo "building Android (release)..." >&2
        ( cd android && ./gradlew :app:assembleRelease ) 1>&2
      else
        echo "install-only: reusing prior APK for $DEVICE_ID" >&2
      fi
      install_android_artifact "$(golem_pick_apk android/app/build/outputs/apk/release)"
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
      app=$(golem_pick_app build/eas/ios-extract 3)
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
