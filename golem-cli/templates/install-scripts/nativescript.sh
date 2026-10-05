#!/usr/bin/env bash
# golem install script — NativeScript
#
# Invoked by golem before each flow to build and install a NativeScript app
# onto a target simulator/emulator or physical device. Runs from the project
# root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#
# Builds with `ns build`, then installs the artifact that build wrote with
# simctl / devicectl (iOS) or adb (Android). Not `ns build --copy-to`: it
# covers only the device .ipa and has been broken for simulator builds.
#
# Host needs: the NativeScript CLI (a `nativescript` devDependency, or a
# global `ns`), and CocoaPods + Ruby for iOS.
#
# Environment (template config — set via [[apps]] install_env or the shell):
#   BUILD_TYPE       = "debug" (default) | "release" (adds --release)
#   NS_BUILD_ARGS    = extra `ns build` flags, word-split: `--env.*` bundler
#                      flags, or the --key-store-* flags an Android release
#                      build needs for signing
#   DEVELOPMENT_TEAM = Apple team ID for a physical iOS device (--team-id)
#   PROVISION        = provisioning profile for a physical iOS device
#                      (--provision); use this or DEVELOPMENT_TEAM
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
NS_DIR="{{NS_DIR}}"               # path to the NativeScript project (contains nativescript.config.*)
NS_CMD="{{NS_CMD}}"               # NativeScript CLI: npx ns | ns
PM_INSTALL="{{PM_INSTALL}}"       # dependency install: npm install | yarn | pnpm install | bun install

BUILD_TYPE="${BUILD_TYPE:-debug}"
NS_BUILD_ARGS="${NS_BUILD_ARGS:-}"
DEVELOPMENT_TEAM="${DEVELOPMENT_TEAM:-}"
PROVISION="${PROVISION:-}"

BUILD_ARGS=()
case "$BUILD_TYPE" in
  debug)   CONFIGURATION=Debug ;;
  release) CONFIGURATION=Release; BUILD_ARGS+=(--release) ;;
  *)
    echo "error: unknown BUILD_TYPE='$BUILD_TYPE' (expected 'debug' or 'release')" >&2
    exit 1
    ;;
esac
# Word-split on purpose: NS_BUILD_ARGS is a list of flags.
# shellcheck disable=SC2206
if [[ -n "$NS_BUILD_ARGS" ]]; then BUILD_ARGS+=($NS_BUILD_ARGS); fi

cd "$NS_DIR"

{{>helpers}}

{{>deps}}

{{>ios}}

case "$PLATFORM" in
  android)
    APK_ROOT="platforms/android/app/build/outputs/apk"
    if [[ "$MODE" != "install-only" ]]; then
      golem_ensure_deps "$PM_INSTALL"
      BUILD_START_TS=$(date +%s)
      # An up-to-date build leaves the APK unwritten, which the freshness
      # guard cannot tell from a stale one. With the APK gone, `ns build`
      # repackages it.
      rm -rf "${APK_ROOT:?}/$BUILD_TYPE"
      echo "ns build android ($BUILD_TYPE)..." >&2
      # The `+` expansion: bash < 4.4 calls an empty array unbound under
      # `set -u`, and a debug build with no NS_BUILD_ARGS passes no flags.
      $NS_CMD build android ${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"} 1>&2
    else
      echo "install-only: reusing prior APK for $DEVICE_ID" >&2
    fi
    APK=$(golem_pick_apk "$APK_ROOT" "$BUILD_TYPE")
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
      SDK=iphonesimulator
    else
      SDK=iphoneos
      BUILD_ARGS+=(--for-device)
      if [[ -n "$PROVISION" ]]; then BUILD_ARGS+=(--provision "$PROVISION"); fi
      if [[ -n "$DEVELOPMENT_TEAM" ]]; then BUILD_ARGS+=(--team-id "$DEVELOPMENT_TEAM"); fi
    fi
    # The CLI has written its products to both `build/` and `Build/` across
    # versions, so both are searched and cleared, never one assumed.
    PRODUCTS_DIRS=("platforms/ios/build/$CONFIGURATION-$SDK" "platforms/ios/Build/$CONFIGURATION-$SDK")
    if [[ "$MODE" != "install-only" ]]; then
      golem_ensure_deps "$PM_INSTALL"
      BUILD_START_TS=$(date +%s)
      for d in "${PRODUCTS_DIRS[@]}"; do rm -rf "$d"/*.app; done
      echo "ns build ios ($BUILD_TYPE, $SDK)..." >&2
      $NS_CMD build ios ${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"} 1>&2
    else
      echo "install-only: reusing prior build for $DEVICE_ID" >&2
    fi
    APP_PATH=""
    for d in "${PRODUCTS_DIRS[@]}"; do
      if [[ -z "$APP_PATH" ]]; then APP_PATH=$(golem_pick_app "$d"); fi
    done
    if [[ -z "$APP_PATH" ]]; then
      echo "error: no .app under platforms/ios/{build,Build}/$CONFIGURATION-$SDK (build may have been skipped — re-run without install-only)" >&2
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
