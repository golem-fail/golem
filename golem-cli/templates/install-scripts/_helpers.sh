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
