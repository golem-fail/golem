#!/usr/bin/env bash
# Start the portal page on an ephemeral loopback port and print its URL, which
# the flow captures with `save_to` and hands to `browse_navigate`.
#
# A real origin, not a `data:` URL: cookies, web storage and WebMCP are all
# absent on an opaque origin, so a page served over http://127.0.0.1 is the only
# fixture that can exercise the whole browser surface.
set -euo pipefail

E2E_DIR="$(cd "$(dirname "$0")/.." && pwd)"
STATE_DIR="${TMPDIR:-/tmp}/golem-browse-e2e"
mkdir -p "$STATE_DIR"
PORT_FILE="$STATE_DIR/port"
PID_FILE="$STATE_DIR/pid"

# A stale server from an interrupted run would serve the old page on the old
# port, so start from a clean slate every time.
[ -f "$PID_FILE" ] && kill "$(cat "$PID_FILE")" 2>/dev/null || true
rm -f "$PORT_FILE" "$PID_FILE"

python3 "$E2E_DIR/scripts/serve_page.py" "$E2E_DIR/__pages__/portal" "$PORT_FILE" \
    >/dev/null 2>&1 &
echo $! >"$PID_FILE"

for _ in $(seq 1 100); do
    [ -s "$PORT_FILE" ] && break
    sleep 0.1
done
[ -s "$PORT_FILE" ] || {
    echo "portal server did not report a port within 10s" >&2
    exit 1
}

printf 'http://127.0.0.1:%s/' "$(cat "$PORT_FILE")"
