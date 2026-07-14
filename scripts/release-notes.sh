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

# name<TAB>version for pinned Gradle coordinates in a build.gradle(.kts) on stdin.
# Only `group:artifact:version` literals with a concrete (digit-led) version are
# taken — BOM-managed (`group:artifact`, no version) and `$var` versions are
# skipped. build.gradle lists DIRECT deps, so every match is a direct dep.
gradle_dep_map() {
  grep -oE "['\"][A-Za-z0-9._-]+:[A-Za-z0-9._-]+:[0-9][A-Za-z0-9._-]*['\"]" 2>/dev/null \
    | tr -d "\"'" | awk -F: '{print $1":"$2"\t"$3}'
}

# name<TAB>version for SPM pins in a Package.resolved (v2/v3 JSON) on stdin.
# (No-op today — test-app-b/ios has no SPM manifest — but ready when one lands.)
spm_dep_map() {
  jq -r '(.pins // .object.pins // [])[]
         | ((.identity // .package) + "\t" + (.state.version // .state.revision // empty))' 2>/dev/null || true
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

# ── section 1: authored notes ──────────────────────────────────────────────
# Categories (Keep-a-Changelog + Breaking, tuned to golem's test-writer audience):
#   shown:     breaking, added, improved, fixed, security, deprecated
#   collapsed: internal (dev/contributor: tooling, refactors, CI, test harness)
# Dependencies are a separate collapsed section from the lockfile diff.

breaking=""; added=""; improved=""; fixed=""; security=""; deprecated=""; internal=""

# Map an author-written prefix (or a Conventional-Commit type) to a canonical
# bucket; empty = not a note-worthy type.
canon_type() {
  case "$1" in
    breaking)                      echo breaking ;;
    added|feat)                    echo added ;;
    improved|improve|changed|change|perf) echo improved ;;
    fixed|fix)                     echo fixed ;;
    security|sec)                  echo security ;;
    deprecated|deprecate)          echo deprecated ;;
    internal|dev|chore)            echo internal ;;
    *)                             echo "" ;;
  esac
}

emit_note() {  # <canonical-bucket> <text-with-optional-(#N)>
  case "$1" in
    breaking)   breaking+="- $2"$'\n' ;;
    added)      added+="- $2"$'\n' ;;
    improved)   improved+="- $2"$'\n' ;;
    fixed)      fixed+="- $2"$'\n' ;;
    security)   security+="- $2"$'\n' ;;
    deprecated) deprecated+="- $2"$'\n' ;;
    internal)   internal+="- $2"$'\n' ;;
  esac
}

# Capitalise first letter; leave the rest (identifiers/backticks) untouched.
sentence() { local s="$1"; printf '%s' "${s^}"; }

# Strip trailing whitespace.
trim_trail() { local s="$1"; printf '%s' "${s%"${s##*[![:space:]]}"}"; }

# Remove HTML comments (single- and multi-line) from stdin, so leftover template
# guidance in the block — even a stray `<!-- - feat: example -->` — is ignored
# rather than emitted or counted.
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

  block=""; pr_suffix=""
  if [[ -n "$pr" ]]; then
    body="$(gh pr view "$pr" --repo "$SLUG" --json body -q .body 2>/dev/null || true)"
    block="$(printf '%s\n' "$body" \
      | awk '/<!-- *release-notes *-->/{f=1;next} /<!-- *\/release-notes *-->/{f=0} f' \
      | strip_comments)"
    # Issue links: GitHub closing keywords in the PR body → "closes #N".
    closes="$(printf '%s\n' "$body" \
      | grep -oiE '(close[sd]?|fix(e[sd])?|resolve[sd]?)[[:space:]]+#[0-9]+' \
      | grep -oE '#[0-9]+' | sort -uV | paste -sd' ' - 2>/dev/null || true)"
    refs="#$pr"; [[ -n "$closes" ]] && refs="$refs, closes $closes"
    pr_suffix=" ($refs)"
  fi

  if [[ -n "${block// /}" ]]; then
    # Authored block: one typed line each. Require a non-space char after the
    # type (rejects empty / "- feat:   "); the leading [^space] also strips
    # leading padding from the captured text.
    re_blockline='^[[:space:]]*-[[:space:]]*(breaking|added|feat|improved|improve|changed|change|perf|fixed|fix|security|sec|deprecated|deprecate|internal|dev|chore):[[:space:]]*([^[:space:]].*)$'
    while IFS= read -r line; do
      [[ "$line" =~ $re_blockline ]] || continue
      bucket="$(canon_type "${BASH_REMATCH[1]}")"; [[ -z "$bucket" ]] && continue
      text="$(trim_trail "${BASH_REMATCH[2]}")"
      emit_note "$bucket" "$(sentence "$text")$pr_suffix"
    done <<< "$block"
  else
    # No block → conventional-subject fallback. Only user-facing types map;
    # internal is opt-in via the block (fallback never invents Internal noise).
    is_skippable_subject "$subject" && continue
    clean="${subject%% (#*}"                       # drop trailing (#N)
    re_conv='^([a-z]+)(\([^)]*\))?(!)?:[[:space:]]*([^[:space:]].*)$'
    if [[ "$clean" =~ $re_conv ]]; then
      t="${BASH_REMATCH[1]}"; bang="${BASH_REMATCH[3]}"; desc="$(trim_trail "${BASH_REMATCH[4]}")"
      if [[ -n "$bang" ]]; then bucket=breaking
      else case "$t" in feat) bucket=added ;; fix) bucket=fixed ;; perf) bucket=improved ;; *) bucket="" ;; esac; fi
      [[ -n "$bucket" ]] && emit_note "$bucket" "$(sentence "$desc")$pr_suffix"
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
  mapfile -t GRADLE_FILES < <(git ls-tree -r --name-only "$NEW" 2>/dev/null \
             | grep -E '(^|/)build\.gradle(\.kts)?$' | grep -v '/node_modules/' || true)
  mapfile -t SPM_FILES < <(git ls-tree -r --name-only "$NEW" 2>/dev/null \
             | grep -E '(^|/)Package\.resolved$' | grep -v '/node_modules/' || true)
fi

# Deduped change sets keyed by dep name (same bump across apps → one line).
declare -A RT_CH=() DEV_CH=() TRANS_SEEN=()
transitive=0

diff_lockfile() {  # <ecosystem> <path> [direct-class]
  # direct-class (e.g. "dev"): treat every changed entry as a direct dep of that
  # class — for ecosystems whose file lists only direct deps (gradle build.gradle)
  # or has no separate manifest to intersect (spm); skips the DEPCLASS/transitive
  # logic used for cargo/npm.
  local eco="$1" path="$2" direct_force="${3:-}" oldmap newmap
  case "$eco" in
    cargo)  oldmap="$(git show "${PREV}:${path}" 2>/dev/null | cargo_lock_map  || true)"
            newmap="$(git show "${NEW}:${path}"  2>/dev/null | cargo_lock_map  || true)" ;;
    npm)    oldmap="$(git show "${PREV}:${path}" 2>/dev/null | npm_lock_map    || true)"
            newmap="$(git show "${NEW}:${path}"  2>/dev/null | npm_lock_map    || true)" ;;
    gradle) oldmap="$(git show "${PREV}:${path}" 2>/dev/null | gradle_dep_map  || true)"
            newmap="$(git show "${NEW}:${path}"  2>/dev/null | gradle_dep_map  || true)" ;;
    spm)    oldmap="$(git show "${PREV}:${path}" 2>/dev/null | spm_dep_map     || true)"
            newmap="$(git show "${NEW}:${path}"  2>/dev/null | spm_dep_map     || true)" ;;
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
    cls="${direct_force:-${DEPCLASS[${eco}:${name}]:-}}"
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

# Ecosystems: Cargo + npm (incl. capacitor) diffed against their lockfiles with
# manifest-based direct-only filtering; Gradle (build.gradle version strings, all
# direct) and SPM (Package.resolved, ready — no manifest in test-app-b/ios yet)
# forced direct-dev. NOT yet parsed → such bumps stay invisible:
#   • Gradle version catalogs (libs.versions.toml) — string-literal coords only.
#   • CocoaPods (Podfile.lock) — none in the repo.
#   • Flutter/Dart (pubspec.lock) — for the coming flutter app.
for f in "${CARGO_LOCKS[@]:-}";  do [[ -n "$f" ]] && diff_lockfile cargo  "$f"; done
for f in "${NPM_LOCKS[@]:-}";    do [[ -n "$f" ]] && diff_lockfile npm    "$f"; done
# gradle/spm files list DIRECT deps only (or have no manifest to intersect), so
# every entry is direct — but classify by location: companions/ ships embedded in
# the binary (→ runtime), test-app* is a fixture (→ dev).
for f in "${GRADLE_FILES[@]:-}"; do [[ -n "$f" ]] && diff_lockfile gradle "$f" "$(loc_class "$f")"; done
for f in "${SPM_FILES[@]:-}";    do [[ -n "$f" ]] && diff_lockfile spm    "$f" "$(loc_class "$f")"; done

# ── render ──────────────────────────────────────────────────────────────────

out=""
[[ -n "$breaking"   ]] && out+="## ⚠️ Breaking"$'\n'"$breaking"$'\n'
[[ -n "$added"      ]] && out+="## ✨ Added"$'\n'"$added"$'\n'
[[ -n "$improved"   ]] && out+="## 🚀 Improved"$'\n'"$improved"$'\n'
[[ -n "$fixed"      ]] && out+="## 🐛 Fixed"$'\n'"$fixed"$'\n'
[[ -n "$security"   ]] && out+="## 🔒 Security"$'\n'"$security"$'\n'
[[ -n "$deprecated" ]] && out+="## 🗑️ Deprecated"$'\n'"$deprecated"$'\n'
# Internal = contributor-facing → collapsed, like deps.
[[ -n "$internal"   ]] && out+="<details><summary>🛠 Internal</summary>"$'\n\n'"$internal"$'\n'"</details>"$'\n\n'

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
