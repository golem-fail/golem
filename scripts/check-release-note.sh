#!/usr/bin/env bash
# Validates a PR body's release-notes block. Reads the body on stdin; the
# author and labels come from the environment (AUTHOR, LABELS) so the caller
# does not have to quote a multi-line body into an argument.
#
# Lives here rather than inline in the workflow because it backs a REQUIRED
# status check: a bug in it blocks every PR in the repo, so it needs the same
# test coverage as the rest of the tooling (scripts/tests/release-note-check.test.sh,
# driven by `cargo t` via golem-cli/tests/release_note_check.rs).
#
# Exit 0 = acceptable, 1 = rejected with a ::error:: annotation on stderr.
note() { printf '%s\n' "$1"; }
reject() {
  printf '::error::%s\n' "$1" >&2
  shift
  for line in "$@"; do printf '%s\n' "$line" >&2; done
  exit 1
}

# Reads the PR body on stdin; AUTHOR / LABELS come from the environment.
# `reject` exits, so callers that want to keep going (the test harness) run
# this in a subshell.
check_release_note() {
  # Set here, not at file scope: sourcing this for tests must not turn on
  # `errexit` in the sourcing shell. Callers run it in a subshell, which
  # confines both these options and `reject`'s exit.
  set -euo pipefail
  local AUTHOR="${AUTHOR:-}" LABELS="${LABELS:-}" BODY
  BODY="$(cat)"

  # Exemptions (still report success so the required check is satisfied).
  case "$AUTHOR" in
    *'[bot]') note "Bot PR ($AUTHOR) — deps come from the lockfile diff; exempt."; exit 0 ;;
  esac
  case ",$LABELS," in
    *,no-release-note,*) note "'no-release-note' label — exempt."; exit 0 ;;
  esac

  # Both markers, in order. An unclosed block captures to end-of-body, so any
  # later line that happens to read like a category (a "Testing" section saying
  # `fixed: the flaky test`) would ship as a phantom note.
  # `|| true` is load-bearing under `set -euo pipefail`: a body with no marker
  # makes grep exit 1, which aborted the whole check before it could print the
  # error explaining what was missing.
  open_at="$(printf '%s\n' "$BODY" | grep -n '^[[:space:]]*<!-- *release-notes *-->[[:space:]]*$' | head -1 | cut -d: -f1 || true)"
  close_at="$(printf '%s\n' "$BODY" | grep -n '^[[:space:]]*<!-- *\/release-notes *-->[[:space:]]*$' | head -1 | cut -d: -f1 || true)"
  if [ -n "$open_at" ] && { [ -z "$close_at" ] || [ "$close_at" -lt "$open_at" ]; }; then
    reject "The release-notes block opens with <!-- release-notes --> but never closes." \
      "Close it with <!-- /release-notes --> (note the slash) — see the PR template."
  fi

  # Extract the block interior, then strip HTML comments (single/multi-line) so
  # leftover template guidance never counts as a real note.
  block="$(printf '%s\n' "$BODY" \
    | awk '/^[[:space:]]*<!-- *release-notes *-->[[:space:]]*$/{f=1;next} /^[[:space:]]*<!-- *\/release-notes *-->[[:space:]]*$/{f=0} f' \
    | awk '{ l=$0
             if (inc) { if (l ~ /-->/) { sub(/.*-->/,"",l); inc=0 } else next }
             gsub(/<!--.*-->/,"",l)
             if (l ~ /<!--/) { sub(/<!--.*/,"",l); inc=1 }
             print l }')"

  # The leading `- ` is required: `scripts/sync-pr-notes.sh` only recognises
  # bulleted lines, so a bare entry is invisible to the trailer sync and would be
  # duplicated rather than deduped against. Release-time extraction still accepts
  # both, so merged PRs keep working.
  if ! printf '%s\n' "$block" | grep -qE '^[[:space:]]*-[[:space:]]+(breaking|added|feat|improved|improve|changed|change|perf|fixed|fix|security|sec|deprecated|deprecate|internal|dev|chore):[[:space:]]*[^[:space:]]'; then
    reject "No release-notes block with a bulleted category line (breaking/added/improved/fixed/security/deprecated/internal) was found in the PR description." \
      "Each entry needs a leading '- ', e.g. '- fixed: tap lands correctly under the keyboard'." \
      "Add one (see the PR template), or apply the 'no-release-note' label if this PR ships nothing user-facing."
  fi

  # A heading SHALL introduce the block. The markers are HTML comments, so
  # without one the notes render to a human reviewer as bullets appearing from
  # nowhere mid-body. Purely presentational — the release tooling reads the
  # markers and never the heading — which is why any heading text passes; this
  # checks that a separator EXISTS, not that it is well written.
  heading_before_marker="$(printf '%s\n' "$BODY" \
    | awk '/^[[:space:]]*<!-- *release-notes *-->[[:space:]]*$/{ print last; exit }
           { if ($0 ~ /[^[:space:]]/) last = $0 }')"
  if ! printf '%s\n' "$heading_before_marker" | grep -qE '^[[:space:]]*#{1,6}[[:space:]]+[^[:space:]]'; then
    reject "The release-notes block needs a Markdown heading immediately above <!-- release-notes -->." \
      "The markers are HTML comments, so without one the notes read as stray bullets." \
      "Any heading text works — '## Release notes' is what the PR template uses." \
      "Found instead: ${heading_before_marker:-(nothing before the marker)}"
  fi

  note "✓ release-notes block present, under a heading."
}

# Sourced by the test harness; executed by the workflow.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  check_release_note
fi
