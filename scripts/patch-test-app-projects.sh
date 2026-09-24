#!/usr/bin/env bash
# Re-applies the test-app declarations Tauri gives us nowhere to configure.
#
#   scripts/patch-test-app-projects.sh android|ios [project-dir]
#
# Two declarations, both needed by e2e and neither expressible in
# `tauri.conf.json`:
#
#   • Android `<uses-permission>` — `pm grant` refuses a permission the
#     package never declared, which is what `permissions_*.test.toml` drives.
#     Tauri 2.x has no config for these at all.
#   • The `golem-test://` scheme the `deep_link` flows open. A
#     `plugins.deep-link.mobile` entry is a universal-link associated domain
#     (`host`/`pathPrefix`); the `schemes` key `tauri-plugin-deep-link` reads
#     lives under `desktop` and never reaches mobile. On iOS it is
#     `CFBundleURLTypes`, absent from the generated plist entirely.
#
# The iOS half writes the scheme TWICE, into two generated files, because
# either can be the one that wins. `gen/apple/project.yml` carries an
# xcodegen `info.properties` block naming `Info.plist` as its output, so
# whenever tauri re-runs xcodegen the plist is rebuilt from the yaml and a
# plist-only edit is gone. On a settled tree xcodegen does not re-run, and
# then the plist is the only copy that matters. Writing both is idempotent
# and cheap; writing one is a coin flip — which is why a fresh clone kept
# failing with LSApplicationWorkspaceErrorDomain 115 while the same commit
# worked on a machine that had built before.
#
# These used to be committed files inside an otherwise-ignored `gen/`, which
# gave a clone a PARTIAL project tree — and `tauri android build` refuses one
# of those outright ("delete the gen/android folder and run tauri android
# init"), so a fresh clone could not build at all (#22). Generating the
# project and re-applying the declarations afterwards keeps `gen/` fully
# generated and fully ignored.
#
# Idempotent by construction: every edit is skipped when its result is
# already present, so it is safe to run before every build, which is what
# scripts/install-test-app.sh does. Pure text in, pure text out — no
# PlistBuddy, no XML library — so the whole thing is testable on any host
# (scripts/tests/patch-test-app-projects.test.sh, run by `cargo t`).

set -euo pipefail

PLATFORM="${1:?usage: patch-test-app-projects.sh android|ios [project-dir]}"
PROJECT_DIR="${2:-test-app/src-tauri}"

SCHEME="golem-test"
# The comment pair tauri-plugin-deep-link's build script writes around the
# region it owns. It rewrites everything between them from tauri.conf.json
# on every rebuild of that crate, so nothing we add may live in there.
MARKER="DEEP LINK PLUGIN. AUTO-GENERATED"

# Everything `permissions_*.test.toml` and `add_media.test.toml` grant. The
# photos trio is deliberate: the shorthand normalizer in
# golem-driver/src/android.rs picks one per SDK level (READ_MEDIA_IMAGES on
# 13+, READ_MEDIA_VISUAL_USER_SELECTED on 14+, READ_EXTERNAL_STORAGE below),
# so all three are declared and whichever it asks for is grantable.
PERMISSIONS=(
  android.permission.CAMERA
  android.permission.RECORD_AUDIO
  android.permission.ACCESS_FINE_LOCATION
  android.permission.ACCESS_COARSE_LOCATION
  android.permission.ACCESS_BACKGROUND_LOCATION
  android.permission.READ_MEDIA_IMAGES
  android.permission.READ_MEDIA_VISUAL_USER_SELECTED
  android.permission.READ_EXTERNAL_STORAGE
)

note() { echo "patch-test-app-projects: $*" >&2; }

patch_android() {
  local manifest="$PROJECT_DIR/gen/android/app/src/main/AndroidManifest.xml"
  [[ -f "$manifest" ]] || { echo "error: $manifest not found — run \`tauri android init\` first" >&2; return 1; }

  local tmp; tmp="$(mktemp)"

  # 1. Permissions, inserted after the <manifest> opening tag. Only the
  #    missing ones, so a re-run adds nothing.
  local missing=() perm
  for perm in "${PERMISSIONS[@]}"; do
    grep -q "android:name=\"$perm\"" "$manifest" || missing+=("$perm")
  done
  if (( ${#missing[@]} > 0 )); then
    note "adding ${#missing[@]} uses-permission declaration(s) to AndroidManifest.xml"
    # Passed as one space-separated line: awk -v cannot carry newlines.
    awk -v perms="${missing[*]}" '
      !done && /^<manifest[ >]/ {
        print
        n = split(perms, p, " ")
        for (i = 1; i <= n; i++) print "    <uses-permission android:name=\"" p[i] "\" />"
        done = 1
        next
      }
      { print }
    ' "$manifest" > "$tmp"
    mv "$tmp" "$manifest"; tmp="$(mktemp)"
  fi

  # 2. Drop any custom-scheme <data> line sitting INSIDE the plugin's region.
  #    That is where it used to live, and where the next rebuild of
  #    tauri-plugin-deep-link silently deletes it.
  if awk -v m="$MARKER" -v s="$SCHEME" '
      index($0, m) { inblk = !inblk; next }
      inblk && index($0, "android:scheme=\"" s "\"") { found = 1 }
      END { exit !found }
    ' "$manifest"; then
    note "removing the $SCHEME scheme from inside the auto-generated block"
    awk -v m="$MARKER" -v s="$SCHEME" '
      index($0, m) { inblk = !inblk; print; next }
      inblk && index($0, "android:scheme=\"" s "\"") { next }
      { print }
    ' "$manifest" > "$tmp"
    mv "$tmp" "$manifest"; tmp="$(mktemp)"
  fi

  # 3. Our own intent-filter, outside the region. Self-contained because
  #    Android matches each filter as a whole, and without autoVerify: that
  #    is an App Links mechanism, and a non-http scheme inside a verified
  #    filter risks the filter being rejected outright.
  if grep -q "android:scheme=\"$SCHEME\"" "$manifest"; then
    rm -f "$tmp"
    return 0
  fi
  note "adding the $SCHEME intent-filter to AndroidManifest.xml"
  # Anchor before the plugin's opening marker when the build has already
  # injected it, else before </activity> — `tauri android init` alone emits
  # no markers, so both states are real.
  local anchor='MARKER'
  grep -q "$MARKER" "$manifest" || anchor='ACTIVITY'
  awk -v m="$MARKER" -v anchor="$anchor" -v s="$SCHEME" '
    function emit() {
      print "            <intent-filter>"
      print "                <action android:name=\"android.intent.action.VIEW\" />"
      print "                <category android:name=\"android.intent.category.DEFAULT\" />"
      print "                <category android:name=\"android.intent.category.BROWSABLE\" />"
      print "                <data android:scheme=\"" s "\" android:host=\"*\" />"
      print "            </intent-filter>"
    }
    !done && anchor == "MARKER"   && index($0, m)        { emit(); done = 1 }
    !done && anchor == "ACTIVITY" && /^[[:space:]]*<\/activity>/ { emit(); done = 1 }
    { print }
    END { if (!done) { print "patch-test-app-projects: no anchor found in manifest" > "/dev/stderr"; exit 1 } }
  ' "$manifest" > "$tmp"
  mv "$tmp" "$manifest"
}

# The xcodegen spec tauri generates. Its `info.properties` block is what
# Info.plist is rebuilt from, so the scheme has to be here too.
patch_ios_project_yml() {
  local spec="$PROJECT_DIR/gen/apple/project.yml"
  [[ -f "$spec" ]] || { echo "error: $spec not found — run \`tauri ios init\` first" >&2; return 1; }

  grep -q "CFBundleURLTypes" "$spec" && return 0

  note "registering the $SCHEME URL scheme in project.yml"
  local tmp; tmp="$(mktemp)"
  awk -v s="$SCHEME" '
    { print }
    !done && /^      properties:[[:space:]]*$/ {
      print "        CFBundleURLTypes:"
      print "          - CFBundleURLName: fail.golem.test.deeplink"
      print "            CFBundleTypeRole: Editor"
      print "            CFBundleURLSchemes: [" s "]"
      done = 1
    }
    END { if (!done) { print "patch-test-app-projects: no info.properties block in project.yml" > "/dev/stderr"; exit 1 } }
  ' "$spec" > "$tmp" || { rm -f "$tmp"; return 1; }
  mv "$tmp" "$spec"
}

patch_ios() {
  patch_ios_project_yml || return 1

  local plist="$PROJECT_DIR/gen/apple/golem-test-app_iOS/Info.plist"
  [[ -f "$plist" ]] || { echo "error: $plist not found — run \`tauri ios init\` first" >&2; return 1; }

  if grep -q "<string>$SCHEME</string>" "$plist"; then
    return 0
  fi
  # Tauri never generates CFBundleURLTypes, so finding one we did not write
  # means somebody hand-edited the plist. Merging into it blind would be
  # guesswork; say so instead.
  if grep -q "CFBundleURLTypes" "$plist"; then
    echo "error: $plist already declares CFBundleURLTypes without the $SCHEME scheme — merge it by hand" >&2
    return 1
  fi

  note "registering the $SCHEME URL scheme in Info.plist"
  local tmp; tmp="$(mktemp)"
  # Before the plist's final </dict>. `simctl openurl` fails with
  # LSApplicationWorkspaceErrorDomain 115 without this.
  awk -v s="$SCHEME" '
    { lines[NR] = $0 }
    END {
      for (i = 1; i <= NR; i++) {
        if (i == last_dict) {
          print "\t<key>CFBundleURLTypes</key>"
          print "\t<array>"
          print "\t\t<dict>"
          print "\t\t\t<key>CFBundleURLName</key>"
          print "\t\t\t<string>fail.golem.test.deeplink</string>"
          print "\t\t\t<key>CFBundleTypeRole</key>"
          print "\t\t\t<string>Editor</string>"
          print "\t\t\t<key>CFBundleURLSchemes</key>"
          print "\t\t\t<array>"
          print "\t\t\t\t<string>" s "</string>"
          print "\t\t\t</array>"
          print "\t\t</dict>"
          print "\t</array>"
        }
        print lines[i]
      }
    }
    { if ($0 ~ /^<\/dict>/) last_dict = NR }
  ' "$plist" > "$tmp"
  mv "$tmp" "$plist"
}

case "$PLATFORM" in
  android) patch_android ;;
  ios)     patch_ios ;;
  both)    patch_android; patch_ios ;;
  *) echo "error: unknown platform $PLATFORM (android|ios|both)" >&2; exit 1 ;;
esac
