#!/usr/bin/env bash
set -euo pipefail

# Build + package a static musl Linux `golem` via the Docker build image, into
# the same dist/ artifacts release.sh produces on macOS:
#   dist/golem-<version>-<target>.tar.gz (+ .sha256)
#
# The build runs on linux/amd64 (Android's aapt2 is x86_64-only). Default target
# is x86_64-unknown-linux-musl; override with GOLEM_LINUX_TARGET. On Apple
# Silicon the build runs under emulation (slow); CI amd64 runners are native.
#
# Usage: scripts/build-linux.sh   (then upload dist/* with release.sh's tag, or
#                                   `gh release upload <tag> dist/golem-*linux*`)

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="${GOLEM_LINUX_TARGET:-x86_64-unknown-linux-musl}"
VERSION="$(grep -m1 '^version = "[0-9]*\.[0-9]*\.[0-9]*"' Cargo.toml \
    | sed -E 's/^version = "(.*)"/\1/')"
[ -n "$VERSION" ] || { echo "error: could not read version from Cargo.toml" >&2; exit 1; }

IMAGE="golem-linux:${TARGET}"
TARBALL="golem-${VERSION}-${TARGET}.tar.gz"
DIST="$ROOT/dist"
mkdir -p "$DIST"

command -v docker >/dev/null 2>&1 || { echo "error: docker not found" >&2; exit 1; }

echo "→ building $IMAGE (linux/amd64)…"
docker build --platform linux/amd64 -f docker/linux-build.Dockerfile \
    --build-arg TARGET="$TARGET" -t "$IMAGE" .

# Preflight: refuse to ship a Linux binary that didn't embed the Android
# companion (mirrors release.sh's embed gate). iOS is intentionally absent on
# Linux, so only Android is required here.
echo "→ preflight: Android companion must be embedded…"
# `doctor` exits non-zero when nothing is drivable (no adb in this minimal
# image) — that's expected here, so capture its output and gate only on the
# embed line rather than the exit code.
doctor_out="$(docker run --rm --platform linux/amd64 "$IMAGE" doctor 2>&1 || true)"
if ! printf '%s\n' "$doctor_out" | grep -qE "Android companion.*embedded"; then
    echo "error: Android companion did NOT embed — refusing to package a driverless Linux binary." >&2
    exit 1
fi

echo "→ extracting binary…"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cid="$(docker create --platform linux/amd64 "$IMAGE")"
docker cp "$cid:/golem" "$tmp/golem"
docker rm "$cid" >/dev/null

echo "→ packaging…"
tar czf "$DIST/$TARBALL" -C "$tmp" golem
( cd "$DIST" \
  && { if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$TARBALL"; else sha256sum "$TARBALL"; fi; } \
       > "$TARBALL.sha256" )

echo "✓ packaged $DIST/$TARBALL"
echo "  $(cat "$DIST/$TARBALL.sha256")"
