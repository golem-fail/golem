#!/bin/sh
# golem installer — the sudo-free fallback channel.
#
#   curl -fsSL https://raw.githubusercontent.com/golem-fail/golem/main/scripts/install.sh | sh
#
# Downloads a prebuilt, self-contained `golem` binary (companions baked in) from
# GitHub Releases, verifies its sha256, and installs it to ~/.golem/bin. This is
# the *fallback* — Homebrew and the npm dev-dependency are the recommended
# channels (they manage PATH + updates). "Updating" here = re-running this line.
#
# Environment overrides:
#   GOLEM_VERSION      pin a version (e.g. 0.7.0); default = latest release
#   GOLEM_INSTALL_DIR  install location (default: ~/.golem/bin)
#   GOLEM_BASE_URL     release-asset base (default: GitHub Releases); point at a
#                      mirror or a local dir laid out as <tag>/<asset>
#   GITHUB_TOKEN       optional; used for the latest-version API lookup
#
# POSIX sh only (this runs piped into `sh`): no bashisms.

set -eu

OWNER="golem-fail"
REPO="golem"
BASE_URL="${GOLEM_BASE_URL:-https://github.com/$OWNER/$REPO/releases/download}"
API_URL="${GOLEM_API_URL:-https://api.github.com/repos/$OWNER/$REPO}"
INSTALL_DIR="${GOLEM_INSTALL_DIR:-$HOME/.golem/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }

# ── Downloader (curl or wget) ─────────────────────────────────────────────────
if command -v curl >/dev/null 2>&1; then
    HAVE=curl
elif command -v wget >/dev/null 2>&1; then
    HAVE=wget
else
    err "need curl or wget on PATH"
fi

# fetch <url> → stdout (for the JSON API; sends the token if present)
fetch() {
    if [ "$HAVE" = curl ]; then
        if [ -n "${GITHUB_TOKEN:-}" ]; then
            curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" "$1"
        else
            curl -fsSL "$1"
        fi
    else
        wget -qO- "$1"
    fi
}

# download <url> <dest>
download() {
    if [ "$HAVE" = curl ]; then
        curl -fsSL -o "$2" "$1"
    else
        wget -qO "$2" "$1"
    fi
}

# ── Detect target triple ──────────────────────────────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Darwin)
        case "$arch" in
            arm64 | aarch64) TARGET="aarch64-apple-darwin" ;;
            x86_64) err "Intel Macs are not supported — golem ships arm64 only." ;;
            *) err "unsupported macOS architecture: $arch" ;;
        esac
        ;;
    Linux)
        # x86_64 Linux ships a static musl build (Android companion only — iOS is
        # macOS-only). arm64 Linux is not published yet.
        case "$arch" in
            x86_64) TARGET="x86_64-unknown-linux-musl" ;;
            aarch64 | arm64)
                err "Linux arm64 is not published yet (x86_64 only). Build from source: cargo install --path golem-cli" ;;
            *) err "unsupported Linux architecture: $arch" ;;
        esac
        ;;
    *)
        err "unsupported OS: $os"
        ;;
esac

# ── Resolve version ───────────────────────────────────────────────────────────
if [ -n "${GOLEM_VERSION:-}" ]; then
    VERSION="$GOLEM_VERSION"
else
    say "→ resolving latest release…"
    json="$(fetch "$API_URL/releases/latest")" || err "could not query the latest release"
    VERSION="$(printf '%s' "$json" \
        | grep '"tag_name"' | head -1 \
        | sed -E 's/.*"tag_name":[[:space:]]*"v?([^"]+)".*/\1/')"
    [ -n "$VERSION" ] || err "could not parse the latest version from the GitHub API"
fi
TAG="v$VERSION"
ASSET="golem-$VERSION-$TARGET.tar.gz"

# ── Download + verify ─────────────────────────────────────────────────────────
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

url="$BASE_URL/$TAG/$ASSET"
say "→ downloading $ASSET ($VERSION)…"
download "$url" "$tmp/$ASSET" || err "download failed: $url"
download "$url.sha256" "$tmp/$ASSET.sha256" || err "checksum download failed: $url.sha256"

say "→ verifying checksum…"
if command -v shasum >/dev/null 2>&1; then
    ( cd "$tmp" && shasum -a 256 -c "$ASSET.sha256" >/dev/null ) || err "checksum mismatch"
elif command -v sha256sum >/dev/null 2>&1; then
    ( cd "$tmp" && sha256sum -c "$ASSET.sha256" >/dev/null ) || err "checksum mismatch"
else
    say "warning: no shasum/sha256sum found — skipping checksum verification"
fi

# ── Install ───────────────────────────────────────────────────────────────────
tar xzf "$tmp/$ASSET" -C "$tmp"
[ -f "$tmp/golem" ] || err "archive did not contain a 'golem' binary"
mkdir -p "$INSTALL_DIR"
cp "$tmp/golem" "$INSTALL_DIR/golem"
chmod +x "$INSTALL_DIR/golem"

say "✓ installed golem $VERSION → $INSTALL_DIR/golem"

# ── PATH hint + next step ─────────────────────────────────────────────────────
case ":$PATH:" in
    *":$INSTALL_DIR:"*) : ;;
    *)
        say ""
        say "$INSTALL_DIR is not on your PATH. Add it:"
        say "  export PATH=\"$INSTALL_DIR:\$PATH\""
        say "…then add that line to your shell profile (~/.zshrc or ~/.bashrc)."
        ;;
esac

say ""
say "Next: run  golem doctor  to check your device toolchain."
