#!/usr/bin/env bash
# Tests the iOS stale-bundle guards in the Tauri install-script template
# (#189): the narrowed tauri-cli failure tolerance, and the web-asset
# freshness checks.
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

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEMPLATE="$REPO_ROOT/golem-cli/templates/install-scripts/tauri.sh"

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

APP_REL="src-tauri/gen/apple/build/arm64-sim/Test.app"

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
APP="$APP_REL"
make_app() { mkdir -p "\$APP"; printf 'plist\n' > "\$APP/Info.plist"; }
fresh_assets() { touch dist/index.html dist/assets/main.js; }
case "\${FAKE_BEHAVIOUR:-}" in
  ok)
    make_app; fresh_assets ;;
  rename-bug)
    make_app; fresh_assets
    echo "Error failed to rename app /x/Test.app: Directory not empty (os error 66)" >&2
    exit 1 ;;
  other-error)
    make_app; fresh_assets
    echo "error: linking with cc failed: exit status 1" >&2
    exit 1 ;;
  stale-app)
    # The isolating case for the .app-vs-assets comparison: the .app IS
    # newer than build start, so the pre-existing mtime guard passes — it
    # is only older than the assets it should have embedded. The assets
    # are dated forward rather than the .app back, so both clear
    # BUILD_START_TS and only the new check can fire.
    make_app; fresh_assets
    touch -t "\$(date -v+5S +%Y%m%d%H%M.%S)" dist/index.html dist/assets/main.js ;;
  empty-dist)
    make_app
    : > dist/index.html ;;
  stale-dist)
    # .app refreshed, but beforeBuildCommand never re-ran the frontend.
    make_app ;;
esac
exit 0
STUB
chmod +x "$BIN/xcrun" "$BIN/faketauri"

sed -e "s|{{TAURI_DIR}}|app|" \
    -e "s|{{IOS_SCHEME}}|Test_iOS|" \
    -e "s|{{TAURI_CMD}}|$BIN/faketauri|" \
    -e "s|{{PM_INSTALL}}||" \
    "$TEMPLATE" > "$WORK/install.sh"

# A minimal project of the shape the script expects, aged so that anything
# the stub does not touch stays visibly stale.
make_project() {
  local dir="$1"
  mkdir -p "$dir/app/$APP_REL" "$dir/app/dist/assets" "$dir/app/src-tauri"
  printf '{ "build": { "frontendDist": "../dist" } }\n' > "$dir/app/src-tauri/tauri.conf.json"
  printf 'plist\n' > "$dir/app/$APP_REL/Info.plist"
  printf '<!doctype html>\n' > "$dir/app/dist/index.html"
  printf 'bundle\n' > "$dir/app/dist/assets/main.js"
  find "$dir/app" -exec touch -t 202001010000 {} + 2>/dev/null
}

# run_case <name> <behaviour> — sets OUT and RC.
run_case() {
  local dir="$WORK/$1"
  make_project "$dir"
  OUT="$(cd "$dir" && PATH="$BIN:$PATH" FAKE_BEHAVIOUR="$2" \
         bash "$WORK/install.sh" ios SIM com.example.test 2>&1)"
  RC=$?
}

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

echo
echo "  $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
