#!/usr/bin/env bash
# Tests scripts/check-release-note.sh, which backs a REQUIRED status check —
# a bug in it blocks every PR in the repo, so it gets covered like any other
# shipped logic rather than living untested inside a workflow's YAML.
#
# Each case feeds a PR body on stdin and asserts the verdict plus, when it
# rejects, WHICH rejection fired: the ordering between "no notes", "unclosed
# block" and "no heading" is itself behaviour (a heading complaint must never
# mask a missing block, whose message is far more useful).
#
# Run directly or via `cargo t` (golem-cli/tests/release_note_check.rs).

set -uo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/check-release-note.sh"
# Source once and call in a subshell per case. Spawning bash 23 times is what
# pushed this harness over the nextest SLOW threshold under a parallel run —
# the same process-spawn contention #66 removed from the installer tests.
# shellcheck disable=SC1090
source "$SCRIPT"
PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); echo "  ok - $1"; }
no() {
  FAIL=$((FAIL + 1))
  echo "  NOT OK - $1"
  [[ -n "${2:-}" ]] && echo "      $2"
}

# accepts <label> <body>
accepts() {
  local out
  if out=$(AUTHOR="${AUTHOR:-someone}" LABELS="${LABELS:-}" check_release_note <<< "$2" 2>&1); then
    ok "$1"
  else
    no "$1" "expected acceptance, got: $out"
  fi
}

# rejects <label> <body> <needle-in-error>
rejects() {
  local out
  if out=$(AUTHOR="${AUTHOR:-someone}" LABELS="${LABELS:-}" check_release_note <<< "$2" 2>&1); then
    no "$1" "expected rejection, but it passed"
  elif grep -qF -- "$3" <<< "$out"; then
    ok "$1"
  else
    no "$1" "rejected for the wrong reason; wanted '$3', got: $out"
  fi
}

NOTE='- fixed: tap lands correctly under the keyboard'

echo "heading requirement"

accepts "the template's own heading passes" "## Release notes
<!-- release-notes -->
$NOTE
<!-- /release-notes -->"

rejects "a block with no heading is rejected" "Some prose about the change.

<!-- release-notes -->
$NOTE
<!-- /release-notes -->" "needs a Markdown heading"

accepts "blank lines between heading and marker are fine" "## Release notes


<!-- release-notes -->
$NOTE
<!-- /release-notes -->"

accepts "any heading text passes — the rule is a separator, not prose" "### Notes for whoever reads the changelog
<!-- release-notes -->
$NOTE
<!-- /release-notes -->"

accepts "an h1 passes" "# Release notes
<!-- release-notes -->
$NOTE
<!-- /release-notes -->"

accepts "a heading deeper in the body still counts if it is the nearest line" "## What

Did a thing.

## Release notes
<!-- release-notes -->
$NOTE
<!-- /release-notes -->"

rejects "bold text is not a heading" "**Release notes**
<!-- release-notes -->
$NOTE
<!-- /release-notes -->" "needs a Markdown heading"

rejects "a hash with no space is not a heading" "##Release notes
<!-- release-notes -->
$NOTE
<!-- /release-notes -->" "needs a Markdown heading"

rejects "a heading followed by prose does not count" "## Release notes

Here is what changed:
<!-- release-notes -->
$NOTE
<!-- /release-notes -->" "needs a Markdown heading"

rejects "nothing at all before the marker" "<!-- release-notes -->
$NOTE
<!-- /release-notes -->" "needs a Markdown heading"

echo
echo "the heading rule never masks a more useful error"

rejects "a body with no block reports the missing block, not the heading" "## What

Just prose, no block at all." "No release-notes block with a bulleted category line"

rejects "an unclosed block reports the missing close marker" "## Release notes
<!-- release-notes -->
$NOTE" "never closes"

rejects "a heading with an empty block reports the missing note" "## Release notes
<!-- release-notes -->
<!-- /release-notes -->" "No release-notes block with a bulleted category line"

echo
echo "pre-existing behaviour still holds"

accepts "several notes in one block" "## Release notes
<!-- release-notes -->
- added: a new selector
- internal: tidied the runner
<!-- /release-notes -->"

accepts "category synonyms are accepted" "## Release notes
<!-- release-notes -->
- feat: something new
<!-- /release-notes -->"

rejects "a note without the leading bullet" "## Release notes
<!-- release-notes -->
fixed: no bullet here
<!-- /release-notes -->" "Each entry needs a leading '- '"

rejects "an unknown category" "## Release notes
<!-- release-notes -->
- refactored: not a category
<!-- /release-notes -->" "No release-notes block with a bulleted category line"

accepts "template guidance comments alone do not count as notes, but a real note beside them does" "## Release notes
<!-- release-notes -->
<!-- One line per change (keep the two markers, replace THIS comment).
     - fixed: this is guidance, not a note -->
$NOTE
<!-- /release-notes -->"

rejects "guidance comments with no real note are still rejected" "## Release notes
<!-- release-notes -->
<!-- One line per change.
     - fixed: this is guidance, not a note -->
<!-- /release-notes -->" "No release-notes block with a bulleted category line"

echo
echo "exemptions short-circuit before any parsing"

AUTHOR='dependabot[bot]' accepts "a bot PR with no block at all is exempt" "Bumps rand from 0.8.5 to 0.10.2."
LABELS='no-release-note' accepts "the no-release-note label exempts a bodyless PR" "## What

Docs only."
LABELS='area: cli,no-release-note,lang: rust' accepts "the label is found among others" "Nothing here."
AUTHOR='dependabot[bot]' accepts "a bot PR is exempt from the heading rule too" "<!-- release-notes -->
$NOTE
<!-- /release-notes -->"

echo
echo "$PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
