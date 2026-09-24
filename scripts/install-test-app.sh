#!/usr/bin/env bash
# golem install script for the Tauri test app — fixture glue around the
# rendered template.
#
# Args are the install-script contract: $1 platform, $2 device id, $3 bundle
# id, $4 optional "install-only".
#
# scripts/install-app.sh is kept byte-identical to the shipped
# golem-cli/templates/install-scripts/tauri.sh (a test enforces it), so it
# cannot carry anything specific to this repo's fixture. This wrapper holds
# what is specific, and hands off:
#
#   1. generate the native project if it is missing or half-there
#   2. re-apply the declarations Tauri has no config for (#22)
#   3. exec the rendered template, which builds and installs
#
# Step 1 has to be explicit rather than left to `tauri <platform> build`,
# which inits on its own: its init would run AFTER step 2 and undo it. On
# iOS that is not hypothetical — `tauri ios init` regenerates the plists
# every time it runs.

set -euo pipefail

PLATFORM="${1:?platform required}"
MODE="${4:-}"   # empty | install-only

TAURI_DIR="test-app"
GEN="$TAURI_DIR/src-tauri/gen"

# install-only reuses the previous artifact and runs no build, so there is
# nothing to generate and nothing for a declaration to end up in.
if [[ "$MODE" == "install-only" ]]; then
  exec bash scripts/install-app.sh "$@"
fi

# A marker file that only a complete generation produces. A tree missing it
# is either absent or half-built, and `tauri android build` refuses the
# half-built case outright rather than repairing it — so clear it first.
case "$PLATFORM" in
  android) marker="$GEN/android/app/build.gradle.kts"; platform_dir="$GEN/android" ;;
  ios)     marker="$GEN/apple/golem-test-app.xcodeproj/project.pbxproj"; platform_dir="$GEN/apple" ;;
  *) echo "error: unknown platform $PLATFORM" >&2; exit 1 ;;
esac

if [[ ! -f "$marker" ]]; then
  if [[ -e "$platform_dir" ]]; then
    echo "install-test-app: $platform_dir is incomplete — regenerating" >&2
    rm -rf "$platform_dir"
  fi
  echo "install-test-app: generating the $PLATFORM project..." >&2
  (cd "$TAURI_DIR" && cargo tauri "$PLATFORM" init 1>&2)
fi

bash scripts/patch-test-app-projects.sh "$PLATFORM"

exec bash scripts/install-app.sh "$@"
