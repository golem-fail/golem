#!/usr/bin/env bash
# Tests scripts/sync-pr-notes.sh, which REWRITES a PR description: it merges
# trailer-derived note lines into the release-notes block. A bug here mangles
# what an author wrote, so it needs coverage — it had none.
#
# The script is pure text in/out by design ("body on stdin, lines-file as $1"),
# so every case is a pipe.
#
# Run directly or via `cargo t` (golem-cli/tests/sync_pr_notes.rs).

set -uo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/sync-pr-notes.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); echo "  ok - $1"; }
no() {
  FAIL=$((FAIL + 1))
  echo "  NOT OK - $1"
  [[ -n "${2:-}" ]] && echo "      $2"
}

# sync <body> <lines...> → rewritten body on stdout
sync() {
  local body="$1"; shift
  printf '%s\n' "$@" > "$TMP/lines"
  printf '%s' "$body" | bash "$SCRIPT" "$TMP/lines"
}

has() { if grep -qF -- "$2" <<< "$1"; then ok "$3"; else no "$3" "expected: $2"; fi; }
lacks() { if grep -qF -- "$2" <<< "$1"; then no "$3" "did not expect: $2"; else ok "$3"; fi; }

echo "merging trailer lines into an existing block"

out="$(sync "## Release notes
<!-- release-notes -->
- fixed: the author's own line
<!-- /release-notes -->" "- added: a synced line")"
has "$out" "- fixed: the author's own line" "an author line survives"
has "$out" "- added: a synced line" "a synced line is added"

out="$(sync "## Release notes
<!-- release-notes -->
- fixed: already here
<!-- /release-notes -->" "- fixed: already here")"
if [[ "$(grep -cF -- '- fixed: already here' <<< "$out")" == "1" ]]; then
  ok "a duplicate line is not added twice"
else
  no "a duplicate line is not added twice" "$out"
fi

echo
echo "a body with no block gains one"

out="$(sync "## What

Did a thing." "- added: something")"
has "$out" "## Release notes" "a heading is appended with the block"
has "$out" "<!-- release-notes -->" "the opening marker is appended"
has "$out" "<!-- /release-notes -->" "the closing marker is appended"
has "$out" "- added: something" "the note lands inside it"

echo
echo "markers are recognised only when alone on their line (#213)"

# The block that gets rewritten must be the real one, not a sentence mentioning
# the marker — otherwise the rewrite swallows the author's prose.
body="## What

This PR is about the \`<!-- release-notes -->\` marker.

## Release notes
<!-- release-notes -->
- fixed: the real note
<!-- /release-notes -->"
out="$(sync "$body" "- added: a synced line")"
has "$out" "This PR is about the" "prose quoting the marker is not eaten by the rewrite"
has "$out" "- fixed: the real note" "the real note survives"
has "$out" "- added: a synced line" "the synced line joins the real block"

# An author note that quotes the marker must survive the round trip.
out="$(sync "## Release notes
<!-- release-notes -->
- internal: the gate wants a heading above the \`<!-- release-notes -->\` block
<!-- /release-notes -->" "- added: a synced line")"
has "$out" "heading above the" "a note quoting the marker survives the rewrite"
has "$out" "- added: a synced line" "and the synced line still lands"

echo
echo "nothing to do"

out="$(sync "## What

No block, no trailers." )"
has "$out" "No block, no trailers." "a body with no lines and no block is unchanged"
lacks "$out" "<!-- release-notes -->" "and gains no block"

echo
echo "$PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
