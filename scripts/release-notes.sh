#!/usr/bin/env bash
set -euo pipefail

# Generate a GitHub release body from what changed between two tags.
#
#   scripts/release-notes.sh <new-tag> [prev-tag]
#
# Sources (see docs/distribution_plan.md → "Automated release notes"):
#   • Breaking / Features / Fixes — authored, per-line, in a marked block in each
#     merged PR's body:
#         <!-- release-notes -->
#         - feat: short line
#         - fix: short line
#         - breaking: short line
#         <!-- /release-notes -->
#     A PR with no block falls back to its conventional-commit subject (skipping
#     non-user-facing types). One PR can contribute lines to several sections.
#   • Dependency updates — computed from the LOCKFILE DIFF, never commit messages.
#     Only *direct* deps are listed (a crate whose key appears in a manifest's
#     dependency table); everything else collapses to "+N transitive". Runtime vs
#     dev is decided by the manifest SECTION the dep is declared in
#     ([dependencies]/[workspace.dependencies] → runtime; [dev-dependencies] and
#     the whole test-app → dev), so a test-only bump never reads as "runtime".
#
# Prints markdown to stdout; release.sh feeds it to `gh release create/edit
# --notes-file`.

NEW="${1:?usage: release-notes.sh <new-tag> [prev-tag]}"
# Previous release tag by ancestry, excluding SemVer prereleases (v*-rc etc.) so
# a lingering prerelease tag can't skew the range.
PREV="${2:-$(git describe --tags --abbrev=0 --exclude='*-*' "${NEW}^" 2>/dev/null || true)}"

SLUG="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || echo "golem-fail/golem")"

# ── helpers ─────────────────────────────────────────────────────────────────

# Bold the changed part of a version so bump size reads at a glance: bold from
# the first differing SemVer segment onward. Non 3-numeric-segment versions (or
# anything with -pre/+meta) just bold the whole new version.
bold_delta() {
  local old="$1" new="$2"
  if [[ ! "$old" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || ! "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    printf '%s → **%s**' "$old" "$new"; return
  fi
  local IFS=.
  local -a o=($old) n=($new)
  local i cut=3
  for i in 0 1 2; do
    if [[ "${o[i]}" != "${n[i]}" ]]; then cut=$i; break; fi
  done
  local head="" tail=""
  for i in 0 1 2; do
    if (( i < cut )); then head+="${n[i]}."; else tail+="${n[i]}."; fi
  done
  head="${head%.}"; tail="${tail%.}"
  if [[ -z "$head" ]]; then printf '%s → **%s**' "$old" "$new"
  else printf '%s → %s.**%s**' "$old" "$head" "$tail"; fi
}

# name<TAB>version for every [[package]] in a Cargo.lock passed on stdin.
cargo_lock_map() {
  awk '
    /^\[\[package\]\]/ { name=""; ver=""; next }
    /^name = "/  { name=$0; sub(/^name = "/,"",name);    sub(/"$/,"",name) }
    /^version = "/ { ver=$0;  sub(/^version = "/,"",ver); sub(/"$/,"",ver)
                     if (name != "") print name "\t" ver }
  '
}

# name<TAB>version for the direct + transitive packages in a package-lock.json
# passed on stdin (npm lockfileVersion 2/3: keys under .packages).
npm_lock_map() {
  jq -r '.packages | to_entries[]
         | select(.key != "")
         | ((.key | sub("^.*node_modules/";"")) + "\t" + (.value.version // empty))' 2>/dev/null || true
}

# Emit "rt <key>" / "dev <key>" for each dependency key declared in the Cargo.toml
# manifests passed as args (read from the $NEW tag). Section decides the class:
#   [dependencies] / [workspace.dependencies] / [*.dependencies]            → rt
#   [dev-dependencies] / [build-dependencies] / [*.dev-dependencies]        → dev
cargo_manifest_keys() {
  local f
  for f in "$@"; do
    git show "${NEW}:${f}" 2>/dev/null || true
  done | awk '
    /^\[/ {
      sec=$0; gsub(/[][]/,"",sec); cls="";
      if (sec ~ /(^|\.)(dev|build)-dependencies$/) cls="dev";
      else if (sec ~ /(^|\.)dependencies$/) cls="rt";
      # inline table header form: [dependencies.foo] / [dev-dependencies.foo]
      if (sec ~ /^dependencies\./)          { print "rt "  substr(sec,14); cls=""; }
      else if (sec ~ /^dev-dependencies\./) { print "dev " substr(sec,18); cls=""; }
      next
    }
    cls!="" && /^[A-Za-z0-9_.-]+[ \t]*[=.]/ {
      key=$0; sub(/[ \t]*[=.].*$/,"",key); gsub(/[ \t]/,"",key);
      if (key != "") print cls " " key
    }
  '
}

# npm dep keys from test-app/package.json (dependencies + devDependencies) — all
# classed dev (the test-app is a test fixture, never shipped).
npm_manifest_keys() {
  git show "${NEW}:test-app/package.json" 2>/dev/null \
    | jq -r '((.dependencies//{}) + (.devDependencies//{})) | keys[]' 2>/dev/null || true
}

# ── section 1: authored notes (Breaking / Features / Fixes) ─────────────────

breaking=""; features=""; fixes=""

emit_note() {  # <type> <text-with-optional-(#N)>
  case "$1" in
    breaking) breaking+="- $2"$'\n' ;;
    feat)     features+="- $2"$'\n' ;;
    fix)      fixes+="- $2"$'\n' ;;
  esac
}

# Capitalise first letter; leave the rest (identifiers/backticks) untouched.
sentence() { local s="$1"; printf '%s' "${s^}"; }

# Conventional-commit types that never produce a user-facing note.
is_skippable_subject() {
  local re_skip='^(chore|ci|docs|test|refactor|style|build|revert)([(:])'
  local re_deps='^(chore|build)\(deps'
  [[ "$1" =~ $re_skip ]] || [[ "$1" =~ $re_deps ]]
}

if [[ -n "$PREV" ]]; then
  RANGE="${PREV}..${NEW}"
else
  RANGE="$NEW"  # first release: whole history
fi

while IFS= read -r subject; do
  [[ -z "$subject" ]] && continue
  pr=""
  if [[ "$subject" =~ \(#([0-9]+)\)[[:space:]]*$ ]]; then
    pr="${BASH_REMATCH[1]}"
  fi

  block=""
  if [[ -n "$pr" ]]; then
    body="$(gh pr view "$pr" --repo "$SLUG" --json body -q .body 2>/dev/null || true)"
    block="$(printf '%s\n' "$body" \
      | awk '/<!-- *release-notes *-->/{f=1;next} /<!-- *\/release-notes *-->/{f=0} f')"
  fi

  if [[ -n "${block// /}" ]]; then
    # Authored block: one typed line each.
    re_blockline='^[[:space:]]*-[[:space:]]*(breaking|feat|fix):[[:space:]]*(.+)$'
    while IFS= read -r line; do
      [[ "$line" =~ $re_blockline ]] || continue
      local_type="${BASH_REMATCH[1]}"; text="${BASH_REMATCH[2]}"
      suffix=""; [[ -n "$pr" ]] && suffix=" (#$pr)"
      emit_note "$local_type" "$(sentence "$text")$suffix"
    done <<< "$block"
  else
    # No block → conventional-subject fallback (skip non-user-facing).
    is_skippable_subject "$subject" && continue
    clean="${subject%% (#*}"                       # drop trailing (#N)
    re_conv='^([a-z]+)(\([^)]*\))?(!)?:[[:space:]]*(.+)$'
    if [[ "$clean" =~ $re_conv ]]; then
      t="${BASH_REMATCH[1]}"; bang="${BASH_REMATCH[3]}"; desc="${BASH_REMATCH[4]}"
      suffix=""; [[ -n "$pr" ]] && suffix=" (#$pr)"
      if [[ -n "$bang" ]]; then emit_note breaking "$(sentence "$desc")$suffix"
      elif [[ "$t" == "feat" ]]; then emit_note feat "$(sentence "$desc")$suffix"
      elif [[ "$t" == "fix"  ]]; then emit_note fix  "$(sentence "$desc")$suffix"
      fi
    fi
  fi
done < <(git log --no-merges --format='%s' "$RANGE" 2>/dev/null || true)

# ── section 2: dependency updates (lockfile diff, direct-only) ──────────────

# Discover manifests + lockfiles at the NEW tag (git-tracked only → never
# node_modules; auto-picks up new test-app*/capacitor apps as they're added).
# Direct deps are classed by manifest section; the workspace's OWN crates (path
# members) are "internal" and dropped (their bump is just the release). Runtime
# wins over dev if a dep is declared both ways.
declare -A DEPCLASS=()
declare -A INTERNAL=()
set_class() {  # eco name class — runtime beats dev; never downgrade
  local k="$1:$2"
  [[ "${DEPCLASS[$k]:-}" == "rt" ]] && return
  DEPCLASS["$k"]="$3"
}

# path → dev if it lives under a test-app* dir, else rt (workspace / npm wrapper).
loc_class() { case "$1" in test-app*) echo dev ;; *) echo rt ;; esac; }

declare -a CARGO_LOCKS=() NPM_LOCKS=()
if [[ -n "$PREV" ]]; then
  # Cargo manifests: workspace (rt/dev by section) vs test-app* (all dev).
  while IFS= read -r m; do
    [[ -z "$m" ]] && continue
    if [[ "$m" == test-app* ]]; then
      while read -r _c key; do [[ -n "$key" ]] && set_class cargo "$key" dev; done \
        < <(cargo_manifest_keys "$m")
    else
      while read -r c key; do [[ -n "$key" ]] && set_class cargo "$key" "$c"; done \
        < <(cargo_manifest_keys "$m")
    fi
    n="$(git show "${NEW}:${m}" 2>/dev/null \
         | awk '/^\[package\]/{p=1} p&&/^name = "/{sub(/^name = "/,"");sub(/"$/,"");print;exit}')"
    [[ -n "$n" ]] && INTERNAL["$n"]=1
  done < <(git ls-tree -r --name-only "$NEW" 2>/dev/null \
             | grep -E '(^|/)Cargo\.toml$' | grep -v '/node_modules/' || true)

  # package.json manifests: test-app* → dev, npm wrapper / root → rt.
  while IFS= read -r m; do
    [[ -z "$m" ]] && continue
    cls="$(loc_class "$m")"
    while IFS= read -r key; do [[ -n "$key" ]] && set_class npm "$key" "$cls"; done \
      < <(git show "${NEW}:${m}" 2>/dev/null \
          | jq -r '((.dependencies//{}) + (.devDependencies//{})) | keys[]' 2>/dev/null || true)
  done < <(git ls-tree -r --name-only "$NEW" 2>/dev/null \
             | grep -E '(^|/)package\.json$' | grep -v '/node_modules/' || true)

  mapfile -t CARGO_LOCKS < <(git ls-tree -r --name-only "$NEW" 2>/dev/null \
             | grep -E '(^|/)Cargo\.lock$' | grep -v '/node_modules/' || true)
  mapfile -t NPM_LOCKS < <(git ls-tree -r --name-only "$NEW" 2>/dev/null \
             | grep -E '(^|/)package-lock\.json$' | grep -v '/node_modules/' || true)
fi

# Deduped change sets keyed by dep name (same bump across apps → one line).
declare -A RT_CH=() DEV_CH=() TRANS_SEEN=()
transitive=0

diff_lockfile() {  # <ecosystem> <path>
  local eco="$1" path="$2" oldmap newmap
  case "$eco" in
    cargo) oldmap="$(git show "${PREV}:${path}" 2>/dev/null | cargo_lock_map || true)"
           newmap="$(git show "${NEW}:${path}"  2>/dev/null | cargo_lock_map || true)" ;;
    npm)   oldmap="$(git show "${PREV}:${path}" 2>/dev/null | npm_lock_map || true)"
           newmap="$(git show "${NEW}:${path}"  2>/dev/null | npm_lock_map || true)" ;;
  esac
  [[ -z "$oldmap$newmap" ]] && return 0

  local names name ov nv line cls
  names="$(printf '%s\n%s\n' "$oldmap" "$newmap" | awk -F'\t' 'NF==2{print $1}' | sort -u)"
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    [[ -n "${INTERNAL[$name]:-}" ]] && continue          # workspace's own crate
    ov="$(printf '%s\n' "$oldmap" | awk -F'\t' -v n="$name" '$1==n{print $2; exit}')"
    nv="$(printf '%s\n' "$newmap" | awk -F'\t' -v n="$name" '$1==n{print $2; exit}')"
    [[ "$ov" == "$nv" ]] && continue                     # unchanged
    cls="${DEPCLASS[${eco}:${name}]:-}"
    if [[ -z "$cls" ]]; then                             # transitive → count once
      [[ -z "${TRANS_SEEN[${eco}:${name}]:-}" ]] && { TRANS_SEEN[${eco}:${name}]=1; transitive=$((transitive+1)); }
      continue
    fi
    if   [[ -z "$ov" ]]; then line="- \`$name\` **added** $nv"
    elif [[ -z "$nv" ]]; then line="- \`$name\` **removed** (was $ov)"
    else line="- \`$name\` $(bold_delta "$ov" "$nv")"; fi
    if [[ "$cls" == "rt" ]]; then [[ -z "${RT_CH[$name]:-}"  ]] && RT_CH["$name"]="$line"
    else                          [[ -z "${DEV_CH[$name]:-}" ]] && DEV_CH["$name"]="$line"; fi
  done <<< "$names"
}

# NOTE(ecosystems not parsed — bumps here are currently INVISIBLE in the notes):
#   • Gradle — test-app-b/android declares pinned deps as version strings in
#     build.gradle (no lockfile). A Dependabot Gradle bump would need a
#     build.gradle version-string differ (BOM/version-catalog aware). Not done.
#   • Flutter (coming) — pubspec.lock (Dart/pub). Needs a pub parser.
#   • iOS native (test-app-b/ios) — no third-party dep manifest (system
#     frameworks only), so nothing to track there.
# Cargo + npm (incl. capacitor, which is npm) are covered above.
for f in "${CARGO_LOCKS[@]}"; do [[ -n "$f" ]] && diff_lockfile cargo "$f"; done
for f in "${NPM_LOCKS[@]}";   do [[ -n "$f" ]] && diff_lockfile npm   "$f"; done

# ── render ──────────────────────────────────────────────────────────────────

out=""
[[ -n "$breaking" ]] && out+="## ⚠️ Breaking"$'\n'"$breaking"$'\n'
[[ -n "$features" ]] && out+="## ✨ Features"$'\n'"$features"$'\n'
[[ -n "$fixes"    ]] && out+="## 🐛 Fixes"$'\n'"$fixes"$'\n'

render_set() {  # name of an assoc array → its values, sorted by key
  local -n _arr="$1"; local k
  for k in $(printf '%s\n' "${!_arr[@]}" | LC_ALL=C sort); do printf '%s\n' "${_arr[$k]}"; done
}
if [[ "${#RT_CH[@]}" -gt 0 || "${#DEV_CH[@]}" -gt 0 || "$transitive" -gt 0 ]]; then
  out+="<details><summary>📦 Dependency updates</summary>"$'\n\n'
  [[ "${#RT_CH[@]}"  -gt 0 ]] && out+="**Runtime (embedded in the binary)**"$'\n'"$(render_set RT_CH)"$'\n\n'
  [[ "${#DEV_CH[@]}" -gt 0 ]] && out+="**Dev / test-app**"$'\n'"$(render_set DEV_CH)"$'\n\n'
  [[ "$transitive" -gt 0 ]] && out+="<sub>+${transitive} transitive</sub>"$'\n'
  out+=$'\n'"</details>"$'\n\n'
fi

if [[ -z "$out" ]]; then
  out="_No user-facing changes._"$'\n\n'
fi

if [[ -n "$PREV" ]]; then
  out+="**Full Changelog**: https://github.com/${SLUG}/compare/${PREV}...${NEW}"$'\n'
fi

printf '%s' "$out"
