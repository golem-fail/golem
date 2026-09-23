#!/usr/bin/env bash
# Tests scripts/patch-test-app-projects.sh — the step that re-applies the
# test app's `uses-permission` lines and its `golem-test://` registration
# after Tauri generates the native projects.
#
# The declarations used to be committed files inside an ignored `gen/`, which
# left a clone with a partial project tree that `tauri android build` refuses
# (#22). They are now re-applied from the script, which makes the script the
# thing that has to be right — and it has two states to get right, because
# `tauri android init` emits no deep-link markers at all while a build
# injects them and rewrites everything between them.
#
# Fixtures are the real generated files, trimmed: the Android manifest as
# `tauri android init` leaves it, and the iOS plist as `tauri ios init`
# leaves it. No Tauri, no device, milliseconds.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PATCHER="$REPO_ROOT/scripts/patch-test-app-projects.sh"
MARKER="DEEP LINK PLUGIN. AUTO-GENERATED"

PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); echo "  ok - $1"; }
no() {
  FAIL=$((FAIL + 1))
  echo "  NOT OK - $1"
  [[ -n "${2:-}" ]] && echo "      $2"
}

check_eq() {
  if [[ "$2" == "$3" ]]; then ok "$1"; else no "$1" "expected '$2', got '$3'"; fi
}
check_contains() {
  # check_contains <label> <file> <needle>
  if grep -qF "$3" "$2"; then ok "$1"; else no "$1" "missing: $3"; fi
}
check_lacks() {
  if grep -qF "$3" "$2"; then no "$1" "unexpectedly present: $3"; else ok "$1"; fi
}

# ── fixtures ────────────────────────────────────────────────────────

# The activity as `tauri android init` writes it: no deep-link markers yet.
ACTIVITY_PLAIN='            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>'

# The activity after a build: the plugin injected its block. `$1` is any
# extra <data> line to place inside it.
activity_with_block() {
  printf '%s\n' "$ACTIVITY_PLAIN"
  printf '%s\n' "            <!-- $MARKER. DO NOT REMOVE. -->"
  cat <<'EOF'
            <intent-filter android:autoVerify="true" >
                <action android:name="android.intent.action.VIEW" />
                <category android:name="android.intent.category.DEFAULT" />
                <category android:name="android.intent.category.BROWSABLE" />
                <data android:scheme="https" />
                <data android:scheme="http" />
EOF
  [[ -n "${1:-}" ]] && printf '%s\n' "$1"
  printf '%s\n' '                <data android:host="*" />'
  printf '%s\n' '            </intent-filter>'
  printf '%s\n' "            <!-- $MARKER. DO NOT REMOVE. -->"
}

# <project-dir> holding a manifest whose activity body is stdin.
new_android() {
  local d body
  d=$(mktemp -d)
  mkdir -p "$d/gen/android/app/src/main"
  body=$(cat)
  {
    echo '<?xml version="1.0" encoding="utf-8"?>'
    echo '<manifest xmlns:android="http://schemas.android.com/apk/res/android">'
    echo '    <uses-permission android:name="android.permission.INTERNET" />'
    echo '    <application>'
    echo '        <activity android:name=".MainActivity">'
    printf '%s\n' "$body"
    echo '        </activity>'
    echo '    </application>'
    echo '</manifest>'
  } > "$d/gen/android/app/src/main/AndroidManifest.xml"
  echo "$d"
}

# The xcodegen spec as `tauri ios init` writes it. Its `info.properties`
# block is what Info.plist is rebuilt from whenever xcodegen re-runs, which
# is why the patcher has to write the scheme into both.
new_ios() {
  local d
  d=$(mktemp -d)
  mkdir -p "$d/gen/apple/golem-test-app_iOS"
  cat > "$d/gen/apple/project.yml" <<'EOF'
name: golem-test-app
targets:
  golem-test-app_iOS:
    type: application
    platform: iOS
    info:
      path: golem-test-app_iOS/Info.plist
      properties:
        LSRequiresIPhoneOS: true
        CFBundleShortVersionString: 0.13.0
        CFBundleVersion: "0.13.0"
EOF
  cat > "$d/gen/apple/golem-test-app_iOS/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>$(PRODUCT_NAME)</string>
	<key>UISupportedInterfaceOrientations</key>
	<array>
		<string>UIInterfaceOrientationPortrait</string>
	</array>
</dict>
</plist>
EOF
  echo "$d"
}

manifest_of() { echo "$1/gen/android/app/src/main/AndroidManifest.xml"; }
plist_of()    { echo "$1/gen/apple/golem-test-app_iOS/Info.plist"; }
spec_of()     { echo "$1/gen/apple/project.yml"; }

# The lines between the two markers — the region the plugin owns and
# rewrites. The patcher must never leave anything of ours in there.
generated_block() {
  awk -v m="$MARKER" 'index($0, m) { inblk = !inblk; next } inblk' "$1"
}

# ── the freshly-generated case: no markers anywhere ─────────────────

echo "android, straight out of \`tauri android init\`:"
d1=$(new_android <<<"$ACTIVITY_PLAIN")
bash "$PATCHER" android "$d1" 2>/dev/null
m1=$(manifest_of "$d1")
check_contains "CAMERA is declared" "$m1" 'android:name="android.permission.CAMERA"'
check_contains "RECORD_AUDIO is declared" "$m1" 'android:name="android.permission.RECORD_AUDIO"'
check_contains "ACCESS_FINE_LOCATION is declared" "$m1" 'android:name="android.permission.ACCESS_FINE_LOCATION"'
check_contains "all three photo permissions are declared" "$m1" 'android:name="android.permission.READ_MEDIA_VISUAL_USER_SELECTED"'
check_contains "the golem-test scheme is declared" "$m1" 'android:scheme="golem-test"'
check_eq "the pre-existing INTERNET permission is kept, not duplicated" \
  "1" "$(grep -c 'android.permission.INTERNET' "$m1")"
check_eq "exactly one golem-test intent-filter is added" \
  "1" "$(grep -c 'android:scheme="golem-test"' "$m1")"
check_lacks "no autoVerify on our filter — that is App Links only" "$m1" 'autoVerify="true"'

before=$(cat "$m1")
bash "$PATCHER" android "$d1" 2>/dev/null
check_eq "running it again changes nothing" "$before" "$(cat "$m1")"

# ── the state that caused #22: scheme inside the plugin's block ──────

echo
echo "android, with the scheme inside the auto-generated block:"
d2=$(new_android < <(activity_with_block '                <data android:scheme="golem-test" />'))
m2=$(manifest_of "$d2")
block_before=$(generated_block "$m2")
bash "$PATCHER" android "$d2" 2>/dev/null
check_eq "the scheme is gone from inside the block" \
  "0" "$(generated_block "$m2" | grep -c 'golem-test')"
check_contains "…and present outside it" "$m2" 'android:scheme="golem-test" android:host="*"'
check_eq "the block keeps the plugin's own entries" \
  "2" "$(generated_block "$m2" | grep -c 'android:scheme=')"
check_eq "exactly one declaration survives overall" \
  "1" "$(grep -c 'android:scheme="golem-test"' "$m2")"

before=$(cat "$m2")
bash "$PATCHER" android "$d2" 2>/dev/null
check_eq "running it again changes nothing" "$before" "$(cat "$m2")"

# ── the steady state: markers present, scheme already outside ───────

echo
echo "android, already correct after a build:"
d3=$(new_android < <(activity_with_block ""))
bash "$PATCHER" android "$d3" 2>/dev/null
m3=$(manifest_of "$d3")
block_after=$(generated_block "$m3")
check_eq "the plugin's block is left byte-identical" \
  "$(activity_with_block "" | awk -v m="$MARKER" 'index($0, m) { inblk = !inblk; next } inblk')" \
  "$block_after"
check_eq "our filter goes outside, not in" \
  "0" "$(generated_block "$m3" | grep -c 'golem-test')"

# ── iOS ─────────────────────────────────────────────────────────────

echo
echo "ios:"
d4=$(new_ios)
p4=$(plist_of "$d4")
s4=$(spec_of "$d4")
bash "$PATCHER" ios "$d4" 2>/dev/null
# project.yml is the one that survives an xcodegen re-run; a plist-only
# edit is rebuilt away, which is what made a fresh clone keep failing with
# LSApplicationWorkspaceErrorDomain 115.
check_contains "project.yml registers the scheme" "$s4" "CFBundleURLSchemes: [golem-test]"
check_eq "…under info.properties, not at the top level" \
  "1" "$(awk '/^      properties:/ { inprops = 1; next } inprops && /CFBundleURLTypes/ { print 1; exit }' "$s4")"
check_contains "project.yml keeps its generated properties" "$s4" "LSRequiresIPhoneOS: true"
check_contains "CFBundleURLTypes is added" "$p4" "CFBundleURLTypes"
check_contains "the golem-test scheme is registered" "$p4" "<string>golem-test</string>"
check_eq "the plist still closes correctly" "</plist>" "$(tail -1 "$p4")"
check_eq "the scheme lands before the root </dict>" \
  "1" "$(awk '/<string>golem-test<\/string>/ { found = NR } /^<\/dict>/ { close_dict = NR } END { print (found < close_dict) ? 1 : 0 }' "$p4")"
if command -v plutil >/dev/null 2>&1; then
  if plutil -lint "$p4" >/dev/null 2>&1; then ok "the patched plist is valid"; else no "the patched plist is valid"; fi
fi

before=$(cat "$p4")
bash "$PATCHER" ios "$d4" 2>/dev/null
check_eq "running it again changes nothing" "$before" "$(cat "$p4")"

# A CFBundleURLTypes we did not write means someone hand-edited the plist.
# Merging into it blind would be guesswork, so the patcher must refuse.
d5=$(new_ios)
p5=$(plist_of "$d5")
perl -0pi -e 's|</dict>\n</plist>|\t<key>CFBundleURLTypes</key>\n\t<array/>\n</dict>\n</plist>|' "$p5"
bash "$PATCHER" ios "$d5" >/dev/null 2>&1
check_eq "a foreign CFBundleURLTypes is refused, not merged into" "1" "$?"

# ── missing project ─────────────────────────────────────────────────

echo
echo "missing project:"
d6=$(mktemp -d)
bash "$PATCHER" android "$d6" >/dev/null 2>&1
check_eq "a missing manifest fails loudly" "1" "$?"
bash "$PATCHER" ios "$d6" >/dev/null 2>&1
check_eq "a missing project.yml fails loudly" "1" "$?"

d7=$(new_ios)
rm "$(plist_of "$d7")"
bash "$PATCHER" ios "$d7" >/dev/null 2>&1
check_eq "a missing plist fails loudly" "1" "$?"

echo
echo "$PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
