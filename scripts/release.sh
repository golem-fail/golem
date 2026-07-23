#!/usr/bin/env bash
set -euo pipefail

# Local-first release: build a self-contained `golem` binary, verify both
# companions actually embedded, package + checksum, and (optionally) upload to a
# GitHub Release. Runnable by hand on a mac; CI is just an optional wrapper over
# this same script (see docs/distribution_plan.md, decision 4).
#
# Naming convention — EVERY downstream channel (curl installer, Homebrew tap,
# npm wrapper, the setup-golem Action) keys off these exact names, so treat them
# as a stable contract:
#
#   golem-<version>-<target-triple>.tar.gz        (gzipped tar; contains only the
#                                                   `golem` binary — companions
#                                                   are baked inside it)
#   golem-<version>-<target-triple>.tar.gz.sha256 (shasum -a 256 format; the
#                                                   payload line names the bare
#                                                   tarball so `shasum -c` works
#                                                   from the download dir)
#
#   e.g. golem-0.7.0-aarch64-apple-darwin.tar.gz
#
# Usage:
#   scripts/release.sh [--tag <tag>] [--draft] [--prerelease]
#                      [--notes <text>] [--no-upload] [--formula <path>]
#
#   --tag <tag>     release tag to upload to (default: v<version-from-Cargo.toml>)
#   --draft         create the release as a draft (only when creating it)
#   --prerelease    mark the release as a prerelease (only when creating it)
#   --notes <text>  release notes body (only when creating the release)
#   --no-upload     build + package + checksum only; skip all GitHub interaction
#   --formula <path> rewrite the version/url/sha256 lines of a Homebrew formula
#                    in place so the tap stays in sync with this release
#
# Idempotent: re-running rebuilds (cheaply, from cache), overwrites the local
# dist/ artifacts, creates the release only if absent, and re-uploads assets with
# --clobber.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DRAFT=""
PRERELEASE=""
NOTES=""
NO_UPLOAD=0
TAG=""
FORMULA=""

while [ $# -gt 0 ]; do
    case "$1" in
        --tag) TAG="${2:?--tag needs a value}"; shift 2 ;;
        --draft) DRAFT="--draft"; shift ;;
        --prerelease) PRERELEASE="--prerelease"; shift ;;
        --notes) NOTES="${2:?--notes needs a value}"; shift 2 ;;
        --no-upload) NO_UPLOAD=1; shift ;;
        --formula) FORMULA="${2:?--formula needs a path}"; shift 2 ;;
        -h|--help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "error: unknown argument: $1" >&2; exit 2 ;;
    esac
done

# ── Version + target ────────────────────────────────────────────────────────
VERSION="$(grep -m1 '^version = "[0-9]*\.[0-9]*\.[0-9]*"' "$ROOT/Cargo.toml" \
    | sed -E 's/^version = "(.*)"/\1/')"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: could not read version from Cargo.toml" >&2
    exit 1
fi
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
: "${TAG:=v$VERSION}"
# owner/repo for the release-asset URL baked into the Homebrew formula.
REPO_SLUG="${GOLEM_REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || echo golem-fail/golem)}"

case "$TARGET" in
    aarch64-apple-darwin) ;;  # the supported release target for this branch
    x86_64-apple-darwin)
        echo "warning: $TARGET is not a distributed target (Intel mac dropped, see plan decision 2)." >&2 ;;
    *)
        echo "warning: $TARGET is outside this branch's supported set (mac arm64 only)." >&2 ;;
esac

# iOS embeds only on macOS targets (plan decision 3); Android embeds everywhere.
NEED_IOS=0
case "$TARGET" in *apple-darwin*) NEED_IOS=1 ;; esac

echo "→ golem $VERSION  target=$TARGET  tag=$TAG"

# ── Release notes (generate up front, fail loud) ───────────────────────────────
# A release without notes is a bug, and generation is cheap, so produce them
# BEFORE the slow build and hard-fail instead of silently shipping a placeholder
# (a late, best-effort generation once let a broken pipe degrade to "Automated
# release …"). Skipped for --no-upload (no release to annotate) or explicit
# --notes. Needs the tag + full history (CI: fetch-depth 0) and bash 4+ (the
# macOS runner's /bin/bash 3.2 lacks the associative arrays the generator uses).
NOTES_FILE=""
if [ "$NO_UPLOAD" -eq 0 ] && [ -z "$NOTES" ]; then
    command -v gh >/dev/null 2>&1 || { echo "error: gh CLI not found (needed for release notes + upload)." >&2; exit 1; }
    NOTES_FILE="$(mktemp)"
    trap 'rm -f "$NOTES_FILE"' EXIT
    if ! "$ROOT/scripts/release-notes.sh" "$TAG" > "$NOTES_FILE"; then
        echo "error: release-notes generation failed for $TAG (see above) — refusing to ship without notes." >&2
        exit 1
    fi
    if [ ! -s "$NOTES_FILE" ]; then
        echo "error: release-notes generation produced no output for $TAG." >&2
        exit 1
    fi
    echo "✓ release notes generated ($(wc -l <"$NOTES_FILE" | tr -d ' ') lines)"
fi

# ── Build ─────────────────────────────────────────────────────────────────────
# Touch build.rs so the build script is guaranteed to re-run and emit its
# out_dir on stdout as JSON — the only reliable, unambiguous handle on THIS
# build's embedded companion artifacts (a fully-cached build emits nothing, and
# stale build-<hash> dirs make globbing ambiguous). Companions themselves stay
# content-hash cached, so this stays cheap.
touch "$ROOT/golem-cli/build.rs"

BUILD_JSON="$(mktemp)"
trap 'rm -f "$BUILD_JSON" ${NOTES_FILE:+"$NOTES_FILE"}' EXIT

echo "→ building release binary (companions build/embed inside)…"
# stdout → JSON (parsed below); stderr → human-rendered progress/errors.
cargo build --release -p golem-cli --bin golem \
    --message-format=json-render-diagnostics >"$BUILD_JSON"

# ── Preflight: both companions must have embedded (non-empty) ──────────────────
OUT_DIR="$(grep '"reason":"build-script-executed"' "$BUILD_JSON" \
    | grep -o '"out_dir":"[^"]*golem-cli-[^"]*"' \
    | tail -1 | sed -E 's/.*"out_dir":"(.*)"/\1/')"
if [ -z "$OUT_DIR" ] || [ ! -d "$OUT_DIR" ]; then
    echo "error: could not resolve the build-script out_dir; cannot verify embeds." >&2
    exit 1
fi

fail_empty() {
    echo "error: $1 embedded EMPTY — the release box is missing its toolchain" >&2
    echo "       ($2). Refusing to ship a binary that can't drive that platform." >&2
    exit 1
}
require_nonempty() { [ -s "$OUT_DIR/$1" ] || fail_empty "$2" "$3"; }

require_nonempty companion-android-test.apk "Android companion (test APK)" "Android SDK + Gradle"
require_nonempty companion-android-main.apk "Android companion (main APK)" "Android SDK + Gradle"
if [ "$NEED_IOS" -eq 1 ]; then
    require_nonempty companion-ios.tar.gz "iOS companion" "Xcode / xcodebuild"
    echo "✓ companions embedded: iOS + Android"
else
    echo "✓ companions embedded: Android (iOS not applicable for $TARGET)"
fi

# ── Package + checksum ─────────────────────────────────────────────────────────
BIN="$ROOT/target/release/golem"
[ -x "$BIN" ] || { echo "error: built binary not found at $BIN" >&2; exit 1; }

DIST="$ROOT/dist"
mkdir -p "$DIST"
TARBALL="golem-$VERSION-$TARGET.tar.gz"
tar czf "$DIST/$TARBALL" -C "$ROOT/target/release" golem

sha256_of() {
    if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1"
    else sha256sum "$1"; fi
}
( cd "$DIST" && sha256_of "$TARBALL" > "$TARBALL.sha256" )

echo "✓ packaged $DIST/$TARBALL"
echo "  $(cat "$DIST/$TARBALL.sha256")"

# ── Homebrew formula sync ──────────────────────────────────────────────────────
# Keep the tap in lock-step with the release: print the version/url/sha256 that
# the formula must carry, and (with --formula) rewrite those three lines in place
# so the tap never drifts from what was actually shipped. Single-target today
# (one url/sha256 line); the deferred Linux work will revisit for per-platform
# blocks.
SHA="$(awk '{print $1; exit}' "$DIST/$TARBALL.sha256")"
ASSET_URL="https://github.com/$REPO_SLUG/releases/download/$TAG/$TARBALL"
echo "→ Homebrew formula fields:"
echo "    version \"$VERSION\""
echo "    url \"$ASSET_URL\""
echo "    sha256 \"$SHA\""
if [ -n "$FORMULA" ]; then
    [ -f "$FORMULA" ] || { echo "error: formula not found: $FORMULA" >&2; exit 1; }
    sed -i.bak -E \
        -e "s|^([[:space:]]*)version \".*\"|\1version \"$VERSION\"|" \
        -e "s|^([[:space:]]*)url \".*\"|\1url \"$ASSET_URL\"|" \
        -e "s|^([[:space:]]*)sha256 \".*\"|\1sha256 \"$SHA\"|" \
        "$FORMULA"
    rm -f "$FORMULA.bak"
    echo "✓ updated formula: $FORMULA"
fi

if [ "$NO_UPLOAD" -eq 1 ]; then
    echo "→ --no-upload: skipping GitHub Release."
    exit 0
fi

# ── Upload ─────────────────────────────────────────────────────────────────────
command -v gh >/dev/null 2>&1 || { echo "error: gh CLI not found (needed to upload)." >&2; exit 1; }

if gh release view "$TAG" >/dev/null 2>&1; then
    echo "→ release $TAG exists; uploading assets (clobber)…"
else
    echo "→ creating release ${TAG}…"
    if [ -n "$NOTES_FILE" ]; then
        gh release create "$TAG" $DRAFT $PRERELEASE --title "golem $TAG" --notes-file "$NOTES_FILE"
    else
        gh release create "$TAG" $DRAFT $PRERELEASE --title "golem $TAG" --notes "${NOTES:-Automated release $TAG.}"
    fi
fi

gh release upload "$TAG" \
    "$DIST/$TARBALL" \
    "$DIST/$TARBALL.sha256" \
    --clobber

echo "✓ uploaded $TARBALL (+ .sha256) to release $TAG"

# ── Release notes ────────────────────────────────────────────────────────────
# Notes were generated up front (fail-loud) and set at create; re-apply here so a
# re-cut onto an EXISTING release also refreshes them. Explicit --notes wins.
if [ -n "$NOTES_FILE" ]; then
    gh release edit "$TAG" --notes-file "$NOTES_FILE" >/dev/null
    echo "✓ release notes applied"
elif [ -n "$NOTES" ]; then
    gh release edit "$TAG" --notes "$NOTES" >/dev/null
fi

gh release view "$TAG" --json url --jq '.url' 2>/dev/null || true
