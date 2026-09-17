#!/usr/bin/env bash
# Tests the freshness gates in the install-script templates.
#
# The templates decide whether to install dependencies and whether to
# regenerate a native project. Those decisions used to be "does the directory
# exist?", which answers yes forever — so a lockfile change built against the
# previous dependency tree and the run reported green. These tests pin the
# replacement: the gates compare what a tree was generated FROM.
#
# The real `npm install` / `expo prebuild` are never run. Each template is
# sourced into a throwaway project with those commands stubbed as recorders,
# so a test asserts on what the script DECIDED to do, in milliseconds.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXPO_TEMPLATE="$REPO_ROOT/golem-cli/templates/install-scripts/expo.sh"
TAURI_TEMPLATE="$REPO_ROOT/golem-cli/templates/install-scripts/tauri.sh"

PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); echo "  ok - $1"; }
no() {
  FAIL=$((FAIL + 1))
  echo "  NOT OK - $1"
  [[ -n "${2:-}" ]] && echo "      $2"
}

check_eq() {
  # check_eq <label> <expected> <actual>
  if [[ "$2" == "$3" ]]; then ok "$1"; else no "$1" "expected '$2', got '$3'"; fi
}

# Extract just the freshness helpers plus the two ensure_* functions from a
# template, so they can be sourced without running the whole install. The
# region is delimited by the template's own section comments.
extract_helpers() {
  # $1 = template path, $2 = last function to include
  awk -v last="$2" '
    /^# ── freshness stamps ─/ { grab = 1 }
    grab { print }
    grab && $0 == "}" && seen_last { exit }
    grab && $0 ~ "^" last "\\(\\) \\{" { seen_last = 1 }
  ' "$1"
}

new_project() {
  local dir
  dir=$(mktemp -d)
  printf '{"name":"x"}' > "$dir/package.json"
  printf '{"lockfileVersion":3}' > "$dir/package-lock.json"
  printf '{"expo":{"name":"x"}}' > "$dir/app.json"
  echo "$dir"
}

# Source the expo helpers into a project with recording stubs, run $1, and
# echo what got invoked (one word per action).
run_expo() {
  local dir="$1" script="$2"
  (
    cd "$dir" || exit 1
    # shellcheck disable=SC1090
    source <(extract_helpers "$EXPO_TEMPLATE" ensure_prebuild)
    PM_INSTALL="record_install"
    PM_RUNNER="record_prebuild"
    record_install() { echo "install" >> "$dir/actions"; mkdir -p node_modules; }
    # `$PM_RUNNER prebuild --platform ios` → record_prebuild prebuild --platform ios
    record_prebuild() { echo "prebuild:$3" >> "$dir/actions"; mkdir -p "$3"; }
    eval "$script"
  )
  cat "$dir/actions" 2>/dev/null | tr '\n' ' ' | sed 's/ $//'
  : > "$dir/actions"
}

echo "expo: dependency freshness"

d=$(new_project)
check_eq "a project with no node_modules installs" \
  "install" "$(run_expo "$d" 'ensure_deps')"
check_eq "an unchanged project installs nothing the second time" \
  "" "$(run_expo "$d" 'ensure_deps')"

printf '{"lockfileVersion":3,"changed":true}' > "$d/package-lock.json"
check_eq "a changed lockfile reinstalls" \
  "install" "$(run_expo "$d" 'ensure_deps')"
check_eq "…and settles again afterwards" \
  "" "$(run_expo "$d" 'ensure_deps')"

printf '{"name":"x","dependencies":{"a":"1"}}' > "$d/package.json"
check_eq "a changed package.json reinstalls" \
  "install" "$(run_expo "$d" 'ensure_deps')"

# Switching package manager must not look unchanged just because the new
# lockfile happens to hash like the old one did.
d2=$(new_project)
run_expo "$d2" 'ensure_deps' > /dev/null
lock_contents=$(cat "$d2/package-lock.json")
rm "$d2/package-lock.json"
printf '%s' "$lock_contents" > "$d2/yarn.lock"
check_eq "swapping lockfile flavour with identical contents reinstalls" \
  "install" "$(run_expo "$d2" 'ensure_deps')"

# A failed install must not be remembered as done.
d3=$(new_project)
(
  cd "$d3" || exit 1
  # shellcheck disable=SC1090
  source <(extract_helpers "$EXPO_TEMPLATE" ensure_prebuild)
  PM_INSTALL="false"
  ensure_deps
) > /dev/null 2>&1
check_eq "a failed install writes no stamp" \
  "absent" "$([[ -f "$d3/node_modules/.golem/deps" ]] && echo present || echo absent)"

echo "expo: prebuild freshness"

d4=$(new_project)
check_eq "a missing native project is generated" \
  "prebuild:ios" "$(run_expo "$d4" 'ensure_prebuild ios')"
check_eq "an unchanged project is not regenerated" \
  "" "$(run_expo "$d4" 'ensure_prebuild ios')"

printf '{"lockfileVersion":3,"moved":true}' > "$d4/package-lock.json"
check_eq "a dependency change regenerates the native project" \
  "prebuild:ios" "$(run_expo "$d4" 'ensure_prebuild ios')"

printf '{"expo":{"name":"renamed"}}' > "$d4/app.json"
check_eq "an app config change regenerates it too" \
  "prebuild:ios" "$(run_expo "$d4" 'ensure_prebuild ios')"

check_eq "the other platform is tracked separately" \
  "prebuild:android" "$(run_expo "$d4" 'ensure_prebuild android')"
check_eq "…and ios stays settled" \
  "" "$(run_expo "$d4" 'ensure_prebuild ios')"

# An existing directory is NOT proof of freshness — the old gate's whole bug.
d5=$(new_project)
mkdir -p "$d5/ios"
check_eq "an existing but unstamped native project is regenerated" \
  "prebuild:ios" "$(run_expo "$d5" 'ensure_prebuild ios')"

# Wiping node_modules takes the prebuild stamp with it, so the native project
# is regenerated rather than trusted against a dependency tree that is gone.
rm -rf "$d5/node_modules"
check_eq "a dependency wipe re-triggers prebuild" \
  "prebuild:ios" "$(run_expo "$d5" 'ensure_prebuild ios')"

echo "tauri: dependency freshness"

run_tauri() {
  local dir="$1" pm="$2"
  (
    cd "$dir" || exit 1
    # shellcheck disable=SC1090
    source <(extract_helpers "$TAURI_TEMPLATE" ensure_deps)
    PM_INSTALL="$pm"
    record_install() { echo "install" >> "$dir/actions"; mkdir -p node_modules; }
    ensure_deps
  )
  cat "$dir/actions" 2>/dev/null | tr '\n' ' ' | sed 's/ $//'
  : > "$dir/actions"
}

d6=$(new_project)
check_eq "a tauri project installs when nothing is installed" \
  "install" "$(run_tauri "$d6" record_install)"
check_eq "…and not again when unchanged" \
  "" "$(run_tauri "$d6" record_install)"
printf '{"lockfileVersion":3,"bumped":true}' > "$d6/package-lock.json"
check_eq "a changed lockfile reinstalls" \
  "install" "$(run_tauri "$d6" record_install)"

# A Tauri app with no JS frontend has nothing to install; the scaffolder
# leaves PM_INSTALL empty and the script must not guess.
d7=$(new_project)
check_eq "an empty PM_INSTALL installs nothing" \
  "" "$(run_tauri "$d7" "")"

d8=$(mktemp -d)
check_eq "a project with no package.json installs nothing" \
  "" "$(run_tauri "$d8" record_install)"

echo
echo "$PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
