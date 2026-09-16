#!/usr/bin/env bash
set -euo pipefail

# Tests release-notes.sh's dependency section against a throwaway git repo:
# fixture lockfiles at v0.0.1, bumped versions at v0.0.2, then assert on what
# the script prints. Driving the real entry point (rather than sourcing the
# parsers) covers the discovery globs and the direct/transitive classification
# too — a missing `mapfile` line is exactly the kind of wiring bug a parser-only
# test passes straight through.
#
# Run directly or via `cargo t` (golem-cli/tests/release_notes.rs).

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/release-notes.sh"
FAILED=0

fail() { printf '  FAIL: %s\n' "$1"; FAILED=1; }
ok()   { printf '  ok: %s\n' "$1"; }

# assert_has <label> <needle> — the needle appears in $OUT.
assert_has() {
  if grep -qF -- "$2" <<< "$OUT"; then ok "$1"; else fail "$1 — expected to find: $2"; fi
}
# assert_lacks <label> <needle>
assert_lacks() {
  if grep -qF -- "$2" <<< "$OUT"; then fail "$1 — did not expect: $2"; else ok "$1"; fi
}
# assert_count <label> <needle> <n>
assert_count() {
  local n; n="$(grep -cF -- "$2" <<< "$OUT" || true)"
  if [[ "$n" == "$3" ]]; then ok "$1"; else fail "$1 — expected $3 occurrence(s) of '$2', got $n"; fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"
git init -q .
git config user.email test@example.com
git config user.name "Release Notes Test"
git config commit.gpgsign false

# ── fixtures at v0.0.1 ──────────────────────────────────────────────────────
# A catalog under companions/ (ships in the binary → runtime) and a build.gradle
# declaring one of the same coordinates literally, so the dedupe is exercised.
mkdir -p companions/android/gradle app/ios test-app-f
cat > companions/android/gradle/libs.versions.toml <<'EOF'
[versions]
coroutines = "1.9.0"
agp = "8.5.2"

[libraries]
kotlinx-coroutines = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version.ref = "coroutines" }
okhttp = { module = "com.squareup.okhttp3:okhttp", version = "4.12.0" }
gson = "com.google.code.gson:gson:2.11.0"
material = { group = "com.google.android.material", name = "material", version = "1.12.0" }

[plugins]
android-application = { id = "com.android.application", version.ref = "agp" }
EOF
cat > companions/android/build.gradle <<'EOF'
dependencies {
    implementation 'com.squareup.okhttp3:okhttp:4.12.0'
}
EOF
cat > app/ios/Podfile.lock <<'EOF'
PODS:
  - Alamofire (5.9.1)
  - Firebase/Core (10.24.0):
    - FirebaseCore (= 10.24.0)
  - FirebaseCore (10.24.0)
  - SwiftyJSON (5.0.2)

DEPENDENCIES:
  - Alamofire (~> 5.9)
  - SwiftyJSON

COCOAPODS: 1.15.2
EOF
cat > app/pubspec.lock <<'EOF'
packages:
  http:
    dependency: "direct main"
    source: hosted
    version: "1.2.2"
  meta:
    dependency: transitive
    source: hosted
    version: "1.15.0"
  test:
    dependency: "direct dev"
    source: hosted
    version: "1.25.8"
sdks:
  dart: ">=3.4.0 <4.0.0"
EOF
git add -A && git commit -qm "fixtures" && git tag v0.0.1

# ── bumps at v0.0.2 ─────────────────────────────────────────────────────────
# coroutines via version.ref, okhttp inline (also in build.gradle), the pods
# direct dep SwiftyJSON + the transitive FirebaseCore, and one pub dep of each
# class. gson / material / the plugin stay put — unchanged deps must not appear.
perl -pi -e 's/coroutines = "1\.9\.0"/coroutines = "1.10.0"/' companions/android/gradle/libs.versions.toml
perl -pi -e 's/okhttp", version = "4\.12\.0"/okhttp", version = "4.12.1"/' companions/android/gradle/libs.versions.toml
perl -pi -e 's/okhttp:4\.12\.0/okhttp:4.12.1/' companions/android/build.gradle
perl -pi -e 's/SwiftyJSON \(5\.0\.2\)/SwiftyJSON (5.0.3)/; s/FirebaseCore \(10\.24\.0\)/FirebaseCore (10.25.0)/' app/ios/Podfile.lock
perl -pi -e 's/"1\.2\.2"/"1.3.0"/; s/"1\.15\.0"/"1.16.0"/; s/"1\.25\.8"/"1.25.9"/' app/pubspec.lock
git add -A && git commit -qm "bump deps" && git tag v0.0.2

OUT="$("$SCRIPT" v0.0.2 v0.0.1)"
printf '%s\n' "$OUT" > "$TMP/out.md"

echo "release-notes.sh — lockfile parsers"

# Gradle version catalog: version.ref indirection resolved, inline version read.
assert_has  "catalog resolves version.ref" 'org.jetbrains.kotlinx:kotlinx-coroutines-core` 1.9.0 → 1.**10.0**'
assert_has  "catalog reads an inline version" 'com.squareup.okhttp3:okhttp` 4.12.0 → 4.12.**1**'
# One bump, one line: the catalog and build.gradle both name okhttp, and both
# parsers key it as `group:artifact`, so it must not report twice.
assert_count "catalog + build.gradle coordinate dedupes" 'com.squareup.okhttp3:okhttp`' 1
assert_lacks "unchanged catalog entries stay out" 'com.google.code.gson:gson'
assert_lacks "unchanged catalog plugin stays out" 'com.android.application'

# CocoaPods: DEPENDENCIES decides direct; everything else is transitive.
assert_has  "pods direct dep is listed" 'SwiftyJSON` 5.0.2 → 5.0.**3**'
assert_lacks "pods transitive dep is not listed by name" 'FirebaseCore`'
assert_lacks "unchanged pod stays out" 'Alamofire`'

# pub: the lockfile's own `dependency:` marking decides the class.
assert_has  "pub direct main is listed" 'http` 1.2.2 → 1.**3.0**'
assert_has  "pub direct dev is listed" 'test` 1.25.8 → 1.25.**9**'
assert_lacks "pub transitive is not listed by name" 'meta`'

# Class grouping: direct main → runtime, direct dev → dev.
RT="$(sed -n '/\*\*Runtime (embedded in the binary)\*\*/,/^$/p' <<< "$OUT")"
DEV="$(sed -n '/\*\*Dev \/ test-app\*\*/,/^$/p' <<< "$OUT")"
if grep -qF 'http`' <<< "$RT"; then ok "pub direct main groups under runtime"
else fail "pub direct main groups under runtime — runtime block was: $RT"; fi
if grep -qF 'test`' <<< "$DEV"; then ok "pub direct dev groups under dev"
else fail "pub direct dev groups under dev — dev block was: $DEV"; fi

# Transitive collapse: FirebaseCore + meta changed but are transitive.
assert_has "transitive bumps collapse to a count" '+2 transitive'

if [[ "$FAILED" -ne 0 ]]; then
  echo
  echo "--- generated notes ---"
  cat "$TMP/out.md"
  exit 1
fi
echo "all assertions passed"
