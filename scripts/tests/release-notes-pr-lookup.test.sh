#!/usr/bin/env bash
set -euo pipefail

# Tests how release-notes.sh decides WHICH PR a commit's notes come from.
#
# The authored `<!-- release-notes -->` block lives in the PR body, so the
# generator has to map each commit back to its PR. A squash merge normally
# appends "(#N)" to the subject — but not when the merge title was edited, and
# the failure is silent and wrong rather than empty: with no number the block
# is never read and the commit subject ships instead, filed under whatever
# bucket its conventional type maps to. That is how #219's `internal:` note
# went out as a `fix(` subject under Fixed.
#
# `gh` is stubbed with per-commit fixtures, so the real script runs against a
# throwaway repo with no network. The stub feeds `--jq` to the real jq, which
# release-notes.sh already depends on, so the filter itself is under test and
# not re-implemented here.
#
# Run directly or via `cargo t` (golem-cli/tests/release_notes.rs).

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/release-notes.sh"
FAILED=0

fail() { printf '  FAIL: %s\n' "$1"; FAILED=1; }
ok()   { printf '  ok: %s\n' "$1"; }

assert_has() {
  if grep -qF -- "$2" <<< "$OUT"; then ok "$1"; else fail "$1 — expected to find: $2"; fi
}
assert_lacks() {
  if grep -qF -- "$2" <<< "$OUT"; then fail "$1 — did not expect: $2"; else ok "$1"; fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export GH_FIXTURES="$TMP/fixtures"
mkdir -p "$GH_FIXTURES" "$TMP/bin" "$TMP/repo"

# ── the gh stub ─────────────────────────────────────────────────────────────
cat > "$TMP/bin/gh" <<'STUB'
#!/usr/bin/env bash
case "${1:-}" in
  repo)
    echo "acme/widget"
    ;;
  pr)   # gh pr view <N> --repo <slug> --json body -q .body
    cat "$GH_FIXTURES/pr-$3.body" 2>/dev/null || true
    ;;
  api)  # gh api repos/<slug>/commits/<sha>/pulls --jq <filter>
    path="$2"; shift 2
    filter='.'
    while [[ $# -gt 0 ]]; do
      [[ "$1" == "--jq" ]] && { filter="$2"; shift; }
      shift
    done
    sha="${path#*/commits/}"; sha="${sha%/pulls}"
    src="$GH_FIXTURES/commit-$sha.json"
    [[ -f "$src" ]] || src="$GH_FIXTURES/none.json"
    jq -r "$filter" < "$src"
    ;;
esac
STUB
chmod +x "$TMP/bin/gh"
echo '[]' > "$GH_FIXTURES/none.json"

# ── the repo ────────────────────────────────────────────────────────────────
cd "$TMP/repo"
git init -q .
git config user.email test@example.com
git config user.name "PR Lookup Test"
git config commit.gpgsign false

commit() {  # commit <subject> → echoes the full sha
  echo "$RANDOM$RANDOM" > file.txt
  git add -A
  git commit -qm "$1"
  git rev-parse HEAD
}

commit "chore: base" > /dev/null
git tag v0.0.1

# 1. The normal case: the squash merge appended the number.
sha_numbered=$(commit "feat(a): subject that should be ignored (#101)")
# 2. The #219 case: no number in the subject, but the commit belongs to a
#    merged PR whose body carries the authored block.
sha_lookup=$(commit "fix(b): subject that should be ignored")
# 3. A commit whose only associated PR was never merged — a PR opened against
#    this commit and closed. Its body must not be used.
sha_unmerged=$(commit "fix(c): subject that should be used")
# 4. A commit with no PR at all.
sha_direct=$(commit "fix(d): direct commit subject")
git tag v0.0.2

cat > "$GH_FIXTURES/pr-101.body" <<'EOF'
## Release notes
<!-- release-notes -->
- added: authored note from pr one oh one
<!-- /release-notes -->
EOF
cat > "$GH_FIXTURES/pr-102.body" <<'EOF'
## Release notes
<!-- release-notes -->
- internal: authored note from pr one oh two
<!-- /release-notes -->
EOF
cat > "$GH_FIXTURES/pr-103.body" <<'EOF'
## Release notes
<!-- release-notes -->
- added: authored note from the unmerged pr
<!-- /release-notes -->
EOF

printf '[{"number":102,"merged_at":"2026-01-01T00:00:00Z"}]\n' > "$GH_FIXTURES/commit-$sha_lookup.json"
printf '[{"number":103,"merged_at":null}]\n' > "$GH_FIXTURES/commit-$sha_unmerged.json"

OUT="$(PATH="$TMP/bin:$PATH" bash "$SCRIPT" v0.0.2 v0.0.1 2>/dev/null)"

echo "release-notes.sh PR resolution:"
assert_has  "a numbered subject uses that PR's authored block" \
  "Authored note from pr one oh one (#101)"
assert_lacks "…and not the commit subject" "subject that should be ignored (#101)"

assert_has  "an unnumbered subject still finds its PR and its block" \
  "Authored note from pr one oh two (#102)"
assert_lacks "…and does not fall back to the commit subject" \
  "Subject that should be ignored"

assert_lacks "an unmerged PR's block is not used" "unmerged pr"
assert_has  "…the commit subject is used instead, linked by sha" \
  "Subject that should be used (${sha_unmerged:0:7})"

assert_has  "a commit with no PR falls back to subject + sha" \
  "Direct commit subject (${sha_direct:0:7})"

echo
[[ "$FAILED" -eq 0 ]] && echo "all passed" || echo "failures above"
exit "$FAILED"
