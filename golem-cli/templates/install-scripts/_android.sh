# ── Android build + install (spliced in by `golem install-script`) ──
# Needs the helpers partial above it.

# First letter upper-cased. Not `${1^}`: that needs bash 4, and macOS ships 3.2.
golem_capitalize() {
  printf '%s%s' "$(printf '%s' "${1:0:1}" | tr '[:lower:]' '[:upper:]')" "${1:1}"
}

# Build a Gradle module with `assemble<Flavor><BuildType>`, then install the
# APK that build wrote with `adb install`.
#
#   golem_android_build_install <gradle_root> <module> <flavor> <build_type> \
#                               <serial> <bundle_id> <mode>
#
# <flavor> may be empty. <mode> is "install-only" to skip the build and
# reuse the previous APK. Every failure returns 1 explicitly: a caller in an
# `if` or `||` turns `set -e` off for the whole function body.
golem_android_build_install() {
  local gradle_root="$1" module="$2" flavor="$3" build_type="$4"
  local serial="$5" bundle_id="$6" mode="$7"
  local apk_root="$gradle_root/$module/build/outputs/apk"
  local variant_dir task build_start apk
  if [[ -n "$flavor" ]]; then
    variant_dir="$flavor/$build_type"
  else
    variant_dir="$build_type"
  fi
  task=":${module}:assemble$(golem_capitalize "$flavor")$(golem_capitalize "$build_type")"

  # Not an `install*` task: gradle would install inside the build, so the
  # freshness guard and the bundle-id check below would never see the APK.
  if [[ "$mode" != "install-only" ]]; then
    build_start=$(date +%s)
    echo "building $task ($gradle_root)..." >&2
    # Gradle leaves an up-to-date APK unwritten, which the freshness guard
    # cannot tell from a stale one. Removing this variant's outputs makes every
    # build write its APK, at the cost of re-running only the packaging.
    rm -rf "${apk_root:?}/$variant_dir"
    ( cd "$gradle_root" && ./gradlew "$task" ) 1>&2 || return 1
  else
    echo "install-only: reusing prior APK for $serial" >&2
  fi

  apk=$(golem_pick_apk "$apk_root" "$variant_dir")
  if [[ -z "$apk" ]]; then
    golem_no_apk_error "$apk_root"
    return 1
  fi
  if [[ "$mode" != "install-only" ]]; then
    golem_require_fresh "$apk" "$build_start" || return 1
  fi

  # The APK's applicationId must be the bundle golem launches. A mismatch
  # installs fine and then fails at launch with nothing pointing here. Skipped
  # when no SDK build-tools are found: a missing check beats a false failure.
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}" aapt2="" apk_id
  if [[ -n "$sdk" ]]; then aapt2=$(golem_newest "$sdk/build-tools" -name aapt2 -type f); fi
  if [[ -n "$aapt2" ]]; then
    apk_id=$("$aapt2" dump packagename "$apk" 2>/dev/null || true)
    if [[ -n "$apk_id" && "$apk_id" != "$bundle_id" ]]; then
      echo "error: $apk has applicationId '$apk_id', but golem launches '$bundle_id'." >&2
      echo "       Fix the bundle in golem.toml, or FLAVOR/BUILD_TYPE (an applicationIdSuffix?)." >&2
      return 1
    fi
  fi

  echo "installing $apk on $serial..." >&2
  adb -s "$serial" install -r "$apk" 1>&2 || return 1
}
