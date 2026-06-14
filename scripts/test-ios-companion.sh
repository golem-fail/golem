#!/usr/bin/env bash
# Run the iOS companion's Swift unit tests (Swift Testing) on a simulator.
#
# These are logic-only unit tests in the GolemRunnerTests bundle — no host app,
# no UI automation. `xcodebuild test` boots a simulator itself; nothing needs to
# be installed first. `cargo t` does NOT cover Swift, so this is the gate for
# changes to companion Swift logic (HTTPServer, HierarchySerializer, ...).
#
# Usage: scripts/test-ios-companion.sh [simulator-udid]
#   With no arg, picks the first available iPhone simulator.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT="$ROOT/companions/ios/GolemRunner.xcodeproj"
SCHEME="GolemRunnerTests"
DERIVED_DATA="${DERIVED_DATA:-$ROOT/companions/ios/build/DerivedData}"

UDID="${1:-}"
if [[ -z "$UDID" ]]; then
  # First available iPhone simulator (any runtime).
  UDID=$(xcrun simctl list devices available --json \
    | python3 -c 'import json,sys
d=json.load(sys.stdin)["devices"]
for rt in d.values():
    for dev in rt:
        if "iPhone" in dev.get("name","") and dev.get("isAvailable"):
            print(dev["udid"]); raise SystemExit')
fi

if [[ -z "$UDID" ]]; then
  echo "error: no available iPhone simulator found (xcrun simctl list devices available)" >&2
  exit 1
fi

echo "running $SCHEME on simulator $UDID..." >&2

xcodebuild test \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath "$DERIVED_DATA" \
  -quiet
