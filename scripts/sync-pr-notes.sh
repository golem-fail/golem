#!/usr/bin/env bash
set -euo pipefail

# Merge author-authored + trailer-derived release-note lines into a PR body's
# `<!-- release-notes -->` block. Pure text in/out (no network) so it's testable:
#
#   printf '%s' "$PR_BODY" | scripts/sync-pr-notes.sh <lines-file>
#
# <lines-file> holds the desired lines (each already "- feat: …" / "- fix: …" /
# "- breaking: …"), typically derived from `Release-Note:` commit trailers.
#
# Behaviour:
#   • Union of the block's existing typed lines + the new lines, deduped by text,
#     existing order preserved (bot ADDS; it doesn't reorder or delete author
#     lines — delete a synced line by dropping the commit trailer).
#   • Block present  → its interior is replaced with the union (guidance comment
#     dropped once real lines exist).
#   • Block absent   → a "## Release notes" block is appended.
#   • No new lines and no existing block → body unchanged.

LINES_FILE="${1:?usage: sync-pr-notes.sh <lines-file> (body on stdin)}"
body="$(cat)"

typed_re='^[[:space:]]*-[[:space:]]*(feat|fix|breaking):[[:space:]]*[^[:space:]]'

# Remove HTML comments (single/multi-line) so leftover template guidance in the
# block is never treated as a real note.
strip_comments() {
  awk '
    {
      l=$0
      if (incomment) { if (l ~ /-->/) { sub(/.*-->/,"",l); incomment=0 } else next }
      gsub(/<!--.*-->/,"",l)
      if (l ~ /<!--/) { sub(/<!--.*/,"",l); incomment=1 }
      print l
    }'
}

# Existing typed lines already in the block (comments stripped).
existing="$(printf '%s\n' "$body" \
  | awk '/<!-- *release-notes *-->/{f=1;next} /<!-- *\/release-notes *-->/{f=0} f' \
  | strip_comments | grep -E "$typed_re" || true)"

# Union: existing first, then new lines whose trimmed text isn't already present.
union_file="$(mktemp)"; trap 'rm -f "$union_file"' EXIT
declare -A seen=()
norm() { sed -E 's/^[[:space:]]*-[[:space:]]*//; s/[[:space:]]+$//' <<<"$1"; }
while IFS= read -r l; do
  [[ -z "$l" ]] && continue
  key="$(norm "$l")"; [[ -n "${seen[$key]:-}" ]] && continue
  seen["$key"]=1; printf '%s\n' "$l" >> "$union_file"
done < <(printf '%s\n' "$existing"; grep -E "$typed_re" "$LINES_FILE" 2>/dev/null || true)

[[ ! -s "$union_file" ]] && { printf '%s' "$body"; exit 0; }   # nothing to write

if grep -qF '<!-- release-notes -->' <<<"$body" && grep -qF '<!-- /release-notes -->' <<<"$body"; then
  # Replace the block interior with the union.
  awk -v uf="$union_file" '
    BEGIN { while ((getline l < uf) > 0) U[n++]=l }
    /<!-- *\/release-notes *-->/ && inblk { for (i=0;i<n;i++) print U[i]; print; inblk=0; next }
    /<!-- *release-notes *-->/  && !inblk { print; inblk=1; next }
    inblk { next }
    { print }
  ' <<<"$body"
else
  # No block → append one.
  printf '%s\n\n## Release notes\n<!-- release-notes -->\n' "$body"
  cat "$union_file"
  printf '%s\n' '<!-- /release-notes -->'
fi
