#!/usr/bin/env bash
# golem install script — Tauri mobile
#
# Invoked by golem before each flow to build and install a Tauri mobile
# app onto a target simulator/emulator or physical device.
# Runs from the project root.
#
# Args:
#   $1 = platform ("ios" or "android")
#   $2 = device UDID (iOS) or serial (Android)
#   $3 = bundle id (from [[flow.apps]] bundle)
#   $4 = "install-only" to skip the build and reuse the previous artifact,
#        or empty for full build+install (default).
#        Golem currently always passes empty; the flag is supported for manual
#        dev-iteration and for a future golem-side build-once optimisation
#        (see roadmap: "Install Cache: Build-Once, Install-to-Many").
#
# Exit 0 on success; nonzero on failure (stderr surfaces to golem).

set -euo pipefail

PLATFORM="${1:?platform required}"
DEVICE_ID="${2:?device id required}"
BUNDLE_ID="${3:?bundle id required}"
MODE="${4:-}"   # empty | install-only

# ── Project config — edit these ─────────────────────────────────────
TAURI_DIR="{{TAURI_DIR}}"               # path to Tauri project (contains src-tauri/)
IOS_SCHEME="{{IOS_SCHEME}}"             # iOS scheme name
TAURI_CMD="{{TAURI_CMD}}"               # tauri CLI runner (npx/yarn/pnpm/bun/cargo tauri)
PM_INSTALL="{{PM_INSTALL}}"             # dependency install: npm install | yarn | pnpm install | bun install

cd "$TAURI_DIR"

# ── freshness stamps ────────────────────────────────────────────────
# Tauri builds the frontend through `beforeBuildCommand` in tauri.conf.json,
# which runs the project's own build script — it never installs dependencies.
# So without this the whole build rests on whatever happens to be in
# node_modules, and a lockfile change is never picked up: the native build
# succeeds against the previous dependency tree and the run reports green.
#
# The stamp records what the installed tree was built FROM, so the gate can
# ask "is it current?" rather than "does it exist?". It lives under
# node_modules, which is already ignored by every project's VCS.
GOLEM_STAMP_DIR="node_modules/.golem"

# Every lockfile flavour, not just this project's, so the stamp stays correct
# if the package manager is switched.
GOLEM_DEP_INPUTS=(package.json package-lock.json yarn.lock pnpm-lock.yaml bun.lockb bun.lock)

# Hash of the named files, in order. A file's NAME is hashed alongside its
# contents so swapping one lockfile flavour for an identical-looking other
# still counts as a change. Missing files contribute nothing.
golem_hash() {
  local f
  for f in "$@"; do
    if [[ -f "$f" ]]; then printf '%s\n' "$f"; cat "$f"; fi
  done | shasum | cut -d' ' -f1
}

# Seconds-since-epoch mtime of a path. BSD and GNU `stat` disagree on the
# flag, and the iOS guards below are useless if the call aborts — which is
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

# Install JS dependencies when the inputs have moved since the last install.
# Skipped entirely when PM_INSTALL is empty — a Tauri app with no JS frontend
# has nothing to install, and guessing would be worse than doing nothing.
ensure_deps() {
  [[ -n "$PM_INSTALL" ]] || return 0
  [[ -f package.json ]] || return 0
  local want stamp
  want=$(golem_hash "${GOLEM_DEP_INPUTS[@]}")
  stamp="$GOLEM_STAMP_DIR/deps"
  if [[ -d node_modules && -f "$stamp" && "$(cat "$stamp" 2>/dev/null)" == "$want" ]]; then
    return 0
  fi
  echo "installing JS dependencies (dependency inputs changed)..." >&2
  $PM_INSTALL 1>&2 || return 1
  # Written only after a successful install, so a failure is retried next
  # run rather than remembered as done — and re-hashed, because package
  # managers rewrite the lockfile as part of installing, which would leave a
  # pre-install hash stale the moment it was written.
  mkdir -p "$GOLEM_STAMP_DIR"
  printf '%s' "$(golem_hash "${GOLEM_DEP_INPUTS[@]}")" > "$stamp"
}

# `install-only` reuses the previous artifact and runs no build, so there is
# nothing for fresh dependencies to feed into.
if [[ "$MODE" != "install-only" ]]; then
  ensure_deps
fi

case "$PLATFORM" in
  ios)
    # Detect simulator vs physical device.
    IS_SIMULATOR=0
    if xcrun simctl list devices --json 2>/dev/null | grep -q "\"$DEVICE_ID\""; then
      IS_SIMULATOR=1
    fi

    if [[ "$MODE" != "install-only" ]]; then
      BUILD_START_TS=$(date +%s)
      echo "building Tauri iOS for $DEVICE_ID..." >&2
      # Clear prior build artifacts. The rename step that tauri-cli does
      # at the end of `ios build` fails with "Directory not empty" if the
      # target-arch dir already exists from a prior run — and the failure
      # is silent under `set +e`. When that happens the .app we pick up
      # below is whatever stale tree was left behind, leading to weeks-
      # old bundles being installed without warning.
      rm -rf src-tauri/gen/apple/build/*.xcarchive
      rm -rf src-tauri/gen/apple/build/arm64-sim
      rm -rf src-tauri/gen/apple/build/x86_64
      rm -rf src-tauri/gen/apple/build/aarch64
      # Tauri 2.x iOS targets: aarch64-sim / x86_64 / aarch64.
      # Known bug: tauri-cli 2.10 + Xcode 26 exits nonzero on a post-archive
      # rename step even after producing a valid signed .app. That ONE
      # failure is tolerated below; every other nonzero exit is fatal.
      #
      # `tee` rather than plain redirection: the log is needed to tell the
      # tolerated failure from the rest, and swallowing a multi-minute
      # build's output until it finishes would be a bad trade for it.
      TAURI_LOG=$(mktemp)
      trap 'rm -f "$TAURI_LOG"' EXIT
      tauri_ios_build() {
        if [[ "$IS_SIMULATOR" == "1" ]]; then
          HOST_ARCH=$(uname -m)
          if [[ "$HOST_ARCH" == "x86_64" ]]; then
            $TAURI_CMD ios build --debug --target x86_64
          else
            $TAURI_CMD ios build --debug --target aarch64-sim
          fi
        else
          $TAURI_CMD ios build --debug --target aarch64
        fi
      }
      set +e
      tauri_ios_build 2>&1 | tee "$TAURI_LOG" >&2
      # PIPESTATUS, not $? — $? is tee's status and is always 0.
      TAURI_EXIT=${PIPESTATUS[0]}
      set -e

      # Fail fast on anything that isn't the known rename bug. Accepting
      # every nonzero exit meant a real build failure continued to the
      # install step and reported whatever .app happened to be lying
      # around.
      if [[ "$TAURI_EXIT" -ne 0 ]]; then
        if grep -qF 'failed to rename app' "$TAURI_LOG" \
           && grep -qF 'Directory not empty' "$TAURI_LOG"; then
          echo "warning: tolerated the known tauri-cli rename failure (exit $TAURI_EXIT)" >&2
        else
          echo "error: tauri ios build exited $TAURI_EXIT, and not with the known" >&2
          echo "       'failed to rename app ... Directory not empty' bug. See the build" >&2
          echo "       output above." >&2
          exit 1
        fi
      fi
    else
      echo "install-only: reusing prior build for $DEVICE_ID" >&2
      TAURI_EXIT=0
    fi

    # Find produced .app. Prefer the per-arch target dir (the canonical
    # output) over the xcarchive copy — when the rename succeeded both
    # exist with the same content, but when it failed the xcarchive copy
    # may be a stale or empty shell.
    if [[ "$IS_SIMULATOR" == "1" ]]; then
      HOST_ARCH=$(uname -m)
      if [[ "$HOST_ARCH" == "x86_64" ]]; then
        TARGET_DIR="src-tauri/gen/apple/build/x86_64"
      else
        TARGET_DIR="src-tauri/gen/apple/build/arm64-sim"
      fi
    else
      TARGET_DIR="src-tauri/gen/apple/build/aarch64"
    fi
    APP_PATH=$(find "$TARGET_DIR" -maxdepth 2 -name "*.app" -type d -print -quit 2>/dev/null)
    if [[ -z "$APP_PATH" ]]; then
      APP_PATH=$(find src-tauri/gen/apple/build -maxdepth 5 -name "*.app" -type d -print -quit)
    fi
    if [[ -z "$APP_PATH" || ! -f "$APP_PATH/Info.plist" ]]; then
      echo "error: tauri build failed (exit $TAURI_EXIT) and no valid .app was produced" >&2
      exit 1
    fi
    # Guard against silent stale-bundle installs: if we ran the build
    # (not install-only) the .app must have been written during this run.
    # Picking up a months-old .app because the rename-step failed silently
    # is what bit us for weeks; this turns it into a loud failure.
    if [[ "$MODE" != "install-only" ]]; then
      APP_MTIME=$(golem_mtime "$APP_PATH")
      if (( APP_MTIME < BUILD_START_TS )); then
        echo "error: .app at $APP_PATH was not refreshed by this build (mtime $APP_MTIME < build start $BUILD_START_TS). The tauri-cli rename likely failed and we'd be installing a stale bundle." >&2
        exit 1
      fi
    fi
    # Web-asset freshness. The bundle is compressed into the Rust binary:
    # a unique string placed in the frontend source appears in the built
    # assets and NOWHERE in the produced .app, so the .app cannot be
    # searched for what it embedded. The inputs are checkable instead —
    # the assets must exist, must have been rebuilt by THIS run, and the
    # .app must be newer than them. That covers a fresh-looking .app built
    # from empty or stale assets without claiming to see inside the blob.
    if [[ "$MODE" != "install-only" ]]; then
      # Same source of truth Tauri itself uses, so no second place to
      # configure. `frontendDist` is relative to src-tauri.
      FRONTEND_DIST=$(sed -n 's/.*"frontendDist"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        src-tauri/tauri.conf.json 2>/dev/null | head -1)
      DIST_DIR="src-tauri/$FRONTEND_DIST"
      if [[ -z "$FRONTEND_DIST" || ! -d "$DIST_DIR" ]]; then
        # A dev-URL or asset-protocol config has no directory to check.
        # Skipping loudly beats hard-failing a project shape that is valid.
        echo "note: no frontendDist directory to verify; skipping the web-asset check" >&2
      else
        if [[ ! -s "$DIST_DIR/index.html" ]]; then
          echo "error: $DIST_DIR/index.html is missing or empty — the .app was built" >&2
          echo "       around an empty web bundle and would install a blank app." >&2
          exit 1
        fi
        # A plain `[[ … ]] && VAR=…` here is a trap: as the loop body's last
        # command it returns 1 whenever the condition is false, and `set -e`
        # then kills the script with no output. Whether that happened
        # depended on `find`'s ordering, so it passed on macOS and failed on
        # Linux. `if` has no such status.
        DIST_NEWEST=0
        while IFS= read -r f; do
          m=$(golem_mtime "$f")
          if (( m > DIST_NEWEST )); then DIST_NEWEST="$m"; fi
        done < <(find "$DIST_DIR" -type f)
        if (( DIST_NEWEST == 0 )) || (( DIST_NEWEST < BUILD_START_TS )); then
          echo "error: no file under $DIST_DIR was written by this build (newest $DIST_NEWEST" >&2
          echo "       < build start $BUILD_START_TS). beforeBuildCommand did not re-run, so the" >&2
          echo "       .app embeds whatever the previous build left behind." >&2
          exit 1
        fi
        if (( APP_MTIME < DIST_NEWEST )); then
          echo "error: .app at $APP_PATH (mtime $APP_MTIME) predates the web assets it should" >&2
          echo "       embed (newest $DIST_NEWEST) — it was linked before they were written." >&2
          exit 1
        fi
      fi
    fi

    if [[ "$IS_SIMULATOR" == "1" ]]; then
      xcrun simctl install "$DEVICE_ID" "$APP_PATH" 1>&2
    elif xcrun devicectl --version >/dev/null 2>&1; then
      xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH" 1>&2
    elif command -v ios-deploy >/dev/null 2>&1; then
      ios-deploy --id "$DEVICE_ID" --bundle "$APP_PATH" --no-wifi 1>&2
    else
      echo "error: need Xcode 15+ (devicectl) or ios-deploy for physical devices" >&2
      exit 1
    fi
    ;;
  android)
    if [[ "$MODE" != "install-only" ]]; then
      echo "building Tauri Android..." >&2
      # Tauri produces a universal APK by default; build without installing.
      $TAURI_CMD android build --debug --apk 1>&2
    else
      echo "install-only: reusing prior APK for $DEVICE_ID" >&2
    fi

    # Find produced APK (-print -quit avoids SIGPIPE under pipefail)
    APK=$(find src-tauri/gen/android/app/build/outputs/apk -name "*.apk" -print -quit)
    if [[ -z "$APK" ]]; then
      echo "error: no APK found (build may have been skipped — re-run without install-only)" >&2
      exit 1
    fi
    adb -s "$DEVICE_ID" install -r "$APK" 1>&2
    ;;
  *)
    echo "error: unknown platform $PLATFORM" >&2
    exit 1
    ;;
esac

echo "installed $BUNDLE_ID on $DEVICE_ID" >&2
