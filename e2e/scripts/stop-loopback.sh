#!/usr/bin/env bash
# Stop the shared e2e loopback server. Runs from teardown, so it must succeed
# even when the flow failed before the server ever started.
set -euo pipefail

STATE_DIR="${TMPDIR:-/tmp}/golem-e2e-loopback"
PID_FILE="$STATE_DIR/pid"

if [ -f "$PID_FILE" ]; then
    kill "$(cat "$PID_FILE")" 2>/dev/null || true
fi
rm -f "$PID_FILE" "$STATE_DIR/port"
