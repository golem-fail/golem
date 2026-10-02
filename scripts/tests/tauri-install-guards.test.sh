#!/usr/bin/env bash
# Tests the stale-artifact guards in the Tauri install-script template
# (#189, #71): on iOS the narrowed tauri-cli failure tolerance and the
# web-asset freshness checks; on Android the APK pick and freshness guard;
# and the BUILD_TYPE / TAURI_BUILD_* mapping onto tauri's build flags.
#
# The TEMPLATE is the thing under test, rendered once with a stub for
# `{{TAURI_CMD}}` — that seam is what lets a case script an arbitrary build
# outcome without Xcode, a simulator, or Tauri. `scripts/install-app.sh` is
# that same template with the repo's values substituted, pinned by
# install_freshness.rs, so testing the template covers both.
#
# The stubs and the rendered script are created ONCE and the behaviour is
# selected per case through `$FAKE_BEHAVIOUR`. Writing a fresh executable
# per case instead cost ~350ms each in exec overhead alone (macOS scans
# newly written binaries), which was most of the harness's runtime.

set -uo pipefail

# The tauri template with its `{{>…}}` partials already spliced, rendered by
# tauri_install_guards.rs through the same code `golem install-script` uses.
TEMPLATE="${TAURI_TEMPLATE:?set TAURI_TEMPLATE; run this through cargo t (tauri_install_guards.rs)}"

PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "  ok - $1"; }
no() {
  FAIL=$((FAIL + 1))
  echo "  NOT OK - $1"
  [[ -n "${2:-}" ]] && echo "      $2"
  return 0
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
BIN="$WORK/bin"
mkdir -p "$BIN"

# Both per-arch dirs, because the script picks by `uname -m`: an arm64 host
# looks in arm64-sim, an x86_64 one (every Linux CI runner) in x86_64. A
# fixture that only made the local arch's dir passed here and failed there.
APP_DIRS=(src-tauri/gen/apple/build/arm64-sim src-tauri/gen/apple/build/x86_64)

# `xcrun`: report the device as a simulator, accept the install.
cat > "$BIN/xcrun" <<'STUB'
#!/usr/bin/env bash
case "${1:-}:${2:-}" in
  simctl:list) printf '{"devices":{"r":[{"udid":"SIM"}]}}\n' ;;
  *) exit 0 ;;
esac
STUB

# Stands in for `{{TAURI_CMD}} ios build`, one branch per way a real build
# can end. Runs with cwd = the Tauri dir, as the script leaves it, and the
# script `rm -rf`s the per-arch dir first — so every branch that should end
# with a .app recreates it, exactly as a real build does.
cat > "$BIN/faketauri" <<STUB
#!/usr/bin/env bash
echo "\$*" > tauri-args
APPS=(${APP_DIRS[*]/%//Test.app})
make_app() {
  local a
  for a in "\${APPS[@]}"; do mkdir -p "\$a"; printf 'plist\n' > "\$a/Info.plist"; done
}
fresh_assets() { touch dist/index.html dist/assets/main.js; }
#
# Assets first, .app second, in that order: a real build writes the web
# bundle and then links around it, and the guard under test asserts the
# .app is the newer of the two. Doing it the other way round passed
# whenever both landed inside one second and failed when they straddled a
# boundary — which under a loaded parallel suite is a coin toss.
# Android: the APK lands where AGP puts a universal build, signed or not.
apk_type=release
for a in "\$@"; do [[ "\$a" == --debug ]] && apk_type=debug; done
apk_dir=src-tauri/gen/android/app/build/outputs/apk/universal/\$apk_type
case "\${FAKE_BEHAVIOUR:-}" in
  android-ok)
    mkdir -p "\$apk_dir"; : > "\$apk_dir/app-universal-\$apk_type.apk" ;;
  android-unsigned)
    mkdir -p "\$apk_dir"; : > "\$apk_dir/app-universal-\$apk_type-unsigned.apk" ;;
  android-stale)
    # A build step that copies an old APK into place rather than writing one.
    mkdir -p "\$apk_dir"; : > "\$apk_dir/app-universal-\$apk_type.apk"
    touch -t 202001010000 "\$apk_dir/app-universal-\$apk_type.apk" ;;
  ok)
    fresh_assets; make_app ;;
  rename-bug)
    fresh_assets; make_app
    echo "Error failed to rename app /x/Test.app: Directory not empty (os error 66)" >&2
    exit 1 ;;
  other-error)
    fresh_assets; make_app
    echo "error: linking with cc failed: exit status 1" >&2
    exit 1 ;;
  stale-app)
    # The isolating case for the .app-vs-assets comparison: the .app IS
    # newer than build start, so the pre-existing mtime guard passes — it
    # is only older than the assets it should have embedded. The assets
    # are dated forward rather than the .app back, so both clear
    # BUILD_START_TS and only the new check can fire.
    make_app; fresh_assets
    ahead=\$(date -v+5S +%Y%m%d%H%M.%S 2>/dev/null || date -d '+5 seconds' +%Y%m%d%H%M.%S)
    touch -t "\$ahead" dist/index.html dist/assets/main.js ;;
  empty-dist)
    make_app
    : > dist/index.html ;;
  stale-dist)
    # .app refreshed, but beforeBuildCommand never re-ran the frontend.
    make_app ;;
esac
exit 0
STUB
# `adb`: record what would be installed.
cat > "$BIN/adb" <<'STUB'
#!/usr/bin/env bash
echo "$*" > adb-args
STUB
chmod +x "$BIN/xcrun" "$BIN/faketauri" "$BIN/adb"

sed -e "s|{{TAURI_DIR}}|app|" \
    -e "s|{{IOS_SCHEME}}|Test_iOS|" \
    -e "s|{{TAURI_CMD}}|$BIN/faketauri|" \
    -e "s|{{PM_INSTALL}}||" \
    "$TEMPLATE" > "$WORK/install.sh"

# A minimal project of the shape the script expects, aged so that anything
# the stub does not touch stays visibly stale.
make_project() {
  local dir="$1"
  mkdir -p "$dir/app/dist/assets" "$dir/app/src-tauri"
  printf '{ "build": { "frontendDist": "../dist" } }\n' > "$dir/app/src-tauri/tauri.conf.json"
  local a
  for a in "${APP_DIRS[@]}"; do
    mkdir -p "$dir/app/$a/Test.app"
    printf 'plist\n' > "$dir/app/$a/Test.app/Info.plist"
  done
  printf '<!doctype html>\n' > "$dir/app/dist/index.html"
  printf 'bundle\n' > "$dir/app/dist/assets/main.js"
  find "$dir/app" -exec touch -t 202001010000 {} + 2>/dev/null
}

# run_case <name> <behaviour> [platform] — sets OUT, RC and CASE_DIR (the
# Tauri dir, where the stubs record their args). Extra `KEY=value` words
# for the script's environment go in $CASE_ENV.
run_case() {
  local dir="$WORK/$1"
  make_project "$dir"
  CASE_DIR="$dir/app"
  # shellcheck disable=SC2086 # CASE_ENV is a list of KEY=value words
  OUT="$(cd "$dir" && env PATH="$BIN:$PATH" FAKE_BEHAVIOUR="$2" ${CASE_ENV:-} \
         bash "$WORK/install.sh" "${3:-ios}" SIM com.example.test 2>&1)"
  RC=$?
}
recorded() { cat "$CASE_DIR/$1" 2>/dev/null; }

echo "tauri install-script iOS guards"

# ── the tauri-cli failure tolerance ────────────────────────────────────
run_case clean ok
[[ $RC -eq 0 ]] && ok "a clean build installs" \
  || no "a clean build installs" "rc=$RC: $OUT"

run_case rename rename-bug
if [[ $RC -eq 0 ]] && grep -qF "tolerated the known tauri-cli rename failure" <<<"$OUT"; then
  ok "the known rename failure is still tolerated"
else
  no "the known rename failure is still tolerated" "rc=$RC: $OUT"
fi

run_case other other-error
if [[ $RC -ne 0 ]] && grep -qF "not with the known" <<<"$OUT"; then
  ok "any other nonzero exit fails fast"
else
  no "any other nonzero exit fails fast" "rc=$RC: $OUT"
fi
grep -qF "linking with cc failed" <<<"$OUT" \
  && ok "the real build error still reaches the user" \
  || no "the real build error still reaches the user" "$OUT"

# ── web-asset freshness ────────────────────────────────────────────────
run_case emptydist empty-dist
if [[ $RC -ne 0 ]] && grep -qF "missing or empty" <<<"$OUT"; then
  ok "an empty index.html fails the install"
else
  no "an empty index.html fails the install" "rc=$RC: $OUT"
fi

run_case staledist stale-dist
if [[ $RC -ne 0 ]] && grep -qF "beforeBuildCommand did not re-run" <<<"$OUT"; then
  ok "assets untouched by this build fail the install"
else
  no "assets untouched by this build fail the install" "rc=$RC: $OUT"
fi

run_case staleapp stale-app
if [[ $RC -ne 0 ]] && grep -qF "predates the web assets" <<<"$OUT"; then
  ok "a .app older than the assets it should embed fails the install"
else
  no "a .app older than the assets it should embed fails the install" "rc=$RC: $OUT"
fi

# ── Android ────────────────────────────────────────────────────────────
echo "tauri install-script Android guards"

run_case adebug android-ok android
if [[ $RC -eq 0 ]] && [[ "$(recorded adb-args)" == *"universal/debug/app-universal-debug.apk" ]]; then
  ok "a debug build installs the debug APK"
else
  no "a debug build installs the debug APK" "rc=$RC adb='$(recorded adb-args)': $OUT"
fi
check_args() {
  if [[ "$(recorded tauri-args)" == "$2" ]]; then ok "$1"; else no "$1" "got '$(recorded tauri-args)'"; fi
}
check_args "debug is the default build type" "android build --debug --apk"

CASE_ENV="BUILD_TYPE=release TAURI_BUILD_CONFIG=ci.json TAURI_BUILD_FEATURES=a,b" \
  run_case arelease android-ok android
if [[ $RC -eq 0 ]] && [[ "$(recorded adb-args)" == *"universal/release/app-universal-release.apk" ]]; then
  ok "a release build installs the release APK"
else
  no "a release build installs the release APK" "rc=$RC adb='$(recorded adb-args)': $OUT"
fi
check_args "release drops --debug and passes --config and --features" \
  "android build --config ci.json --features a,b --apk"

CASE_ENV="BUILD_TYPE=release" run_case aunsigned android-unsigned android
if [[ $RC -ne 0 ]] && grep -qF "only unsigned APKs" <<<"$OUT" && [[ -z "$(recorded adb-args)" ]]; then
  ok "an unsigned-only release build fails, naming the signing problem"
else
  no "an unsigned-only release build fails, naming the signing problem" "rc=$RC: $OUT"
fi

run_case astale android-stale android
if [[ $RC -ne 0 ]] && grep -qF "not refreshed by this build" <<<"$OUT" && [[ -z "$(recorded adb-args)" ]]; then
  ok "an APK this build did not write is not installed"
else
  no "an APK this build did not write is not installed" "rc=$RC: $OUT"
fi

CASE_ENV="BUILD_TYPE=profile" run_case abogus android-ok android
if [[ $RC -ne 0 ]] && grep -qF "unknown BUILD_TYPE" <<<"$OUT"; then
  ok "an unknown BUILD_TYPE fails before building"
else
  no "an unknown BUILD_TYPE fails before building" "rc=$RC: $OUT"
fi

# ── iOS build flags ────────────────────────────────────────────────────
CASE_ENV="BUILD_TYPE=release TAURI_BUILD_FEATURES=a" run_case irelease ok
if [[ $RC -eq 0 ]] && [[ "$(recorded tauri-args)" == "ios build --features a --target "* ]]; then
  ok "an iOS release build drops --debug and passes --features"
else
  no "an iOS release build drops --debug and passes --features" "rc=$RC args='$(recorded tauri-args)': $OUT"
fi

echo
echo "  $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
