#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <new-version|--patch>"
    echo "Example: $0 0.5.0"
    echo "         $0 --patch   # read current version, bump Z (X.Y.Z -> X.Y.Z+1)"
    exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ "$1" = "--patch" ]; then
    CURRENT="$(grep -m1 '^version = "[0-9]*\.[0-9]*\.[0-9]*"' "$ROOT/Cargo.toml" | sed -E 's/^version = "(.*)"/\1/')"
    if ! [[ "$CURRENT" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
        echo "Error: could not read current version from $ROOT/Cargo.toml"
        exit 1
    fi
    NEW_VERSION="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((BASH_REMATCH[3] + 1))"
    echo "Current version $CURRENT -> $NEW_VERSION"
else
    NEW_VERSION="$1"
fi

# Validate version format
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: version must be semver (e.g. 0.5.0)"
    exit 1
fi

echo "Bumping all versions to $NEW_VERSION"

# 1. Workspace root (all member crates inherit from here)
sed -i '' "s/^version = \"[0-9]*\.[0-9]*\.[0-9]*\"/version = \"$NEW_VERSION\"/" \
    "$ROOT/Cargo.toml"

# 2. Test app (excluded from workspace, needs manual update)
sed -i '' "s/^version = \"[0-9]*\.[0-9]*\.[0-9]*\"/version = \"$NEW_VERSION\"/" \
    "$ROOT/test-app/src-tauri/Cargo.toml"

sed -i '' "s/\"version\": \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\": \"$NEW_VERSION\"/" \
    "$ROOT/test-app/package.json" \
    "$ROOT/test-app/src-tauri/tauri.conf.json"

# npm wrapper package (@golem-fail/golem) — its version selects the release
# asset its postinstall downloads, so it MUST track the release version.
sed -i '' "s/\"version\": \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\": \"$NEW_VERSION\"/" \
    "$ROOT/npm/package.json"

sed -i '' "s|<string>[0-9]*\.[0-9]*\.[0-9]*</string>|<string>$NEW_VERSION</string>|" \
    "$ROOT/test-app/src-tauri/gen/apple/golem-test-app_iOS/Info.plist"

# 3. Companion health endpoints
sed -i '' "s/\"version\": \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\": \"$NEW_VERSION\"/" \
    "$ROOT/companions/ios/GolemRunnerUITests/RequestRouter.swift"

sed -i '' "s/\"version\", \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\", \"$NEW_VERSION\"/" \
    "$ROOT/companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java"

# 4. Companion test assertions
sed -i '' "s/\"version\": \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\": \"$NEW_VERSION\"/" \
    "$ROOT/companions/ios/GolemRunnerUITests/GolemRunnerUITests.swift"

sed -i '' "s/\"version\", \"[0-9]*\.[0-9]*\.[0-9]*\"/\"version\", \"$NEW_VERSION\"/" \
    "$ROOT/companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServerTest.java"

# Verify
echo ""
echo "Verifying..."
FILES=(
    "$ROOT/Cargo.toml"
    "$ROOT/test-app/src-tauri/Cargo.toml"
    "$ROOT/test-app/package.json"
    "$ROOT/test-app/src-tauri/tauri.conf.json"
    "$ROOT/npm/package.json"
    "$ROOT/test-app/src-tauri/gen/apple/golem-test-app_iOS/Info.plist"
    "$ROOT/companions/ios/GolemRunnerUITests/RequestRouter.swift"
    "$ROOT/companions/ios/GolemRunnerUITests/GolemRunnerUITests.swift"
    "$ROOT/companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java"
    "$ROOT/companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServerTest.java"
)
FAIL=0
for f in "${FILES[@]}"; do
    if ! grep -q "$NEW_VERSION" "$f"; then
        echo "MISSING: $f"
        FAIL=1
    fi
done
if [ $FAIL -eq 0 ]; then
    echo "All ${#FILES[@]} files contain $NEW_VERSION"
else
    echo "WARNING: some files missing new version"
    exit 1
fi
echo ""
echo "Done. Run 'cargo check' to verify workspace resolution."
