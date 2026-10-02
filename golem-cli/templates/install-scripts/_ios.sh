# ── iOS build + install (spliced in by `golem install-script`) ──────
# Needs the helpers partial above it.

# Install the .app at $2 onto the simulator or physical device $1.
golem_ios_install_app() {
  local udid="$1" app="$2"
  echo "installing $app on $udid..." >&2
  if golem_is_simulator "$udid"; then
    xcrun simctl install "$udid" "$app" 1>&2 || return 1
  elif xcrun devicectl --version >/dev/null 2>&1; then
    xcrun devicectl device install app --device "$udid" "$app" 1>&2 || return 1
  elif command -v ios-deploy >/dev/null 2>&1; then
    ios-deploy --id "$udid" --bundle "$app" --no-wifi 1>&2 || return 1
  else
    echo "error: need Xcode 15+ (devicectl) or ios-deploy to install on physical devices" >&2
    return 1
  fi
}

# Build an Xcode scheme for the target device, then install the .app that
# build wrote.
#
#   golem_ios_build_install <project> <scheme> <configuration> <xcconfig> \
#                           <development_team> <derived_data> <udid> <mode>
#
# <project> is an .xcodeproj or .xcworkspace. <xcconfig> and
# <development_team> may be empty; the team applies to a physical device
# only. <mode> is "install-only" to skip the build and reuse the previous
# .app. Every failure returns 1 explicitly: a caller in an `if` or `||`
# turns `set -e` off for the whole function body.
golem_ios_build_install() {
  local project="$1" scheme="$2" configuration="$3" xcconfig="$4"
  local team="$5" derived_data="$6" udid="$7" mode="$8"
  local is_simulator=0 products_dir build_start app
  local build_args=()

  if [[ "$project" == *.xcworkspace ]]; then
    build_args+=(-workspace "$project")
  else
    build_args+=(-project "$project")
  fi
  build_args+=(-scheme "$scheme" -configuration "$configuration" -derivedDataPath "$derived_data")
  if [[ -n "$xcconfig" ]]; then build_args+=(-xcconfig "$xcconfig"); fi

  if golem_is_simulator "$udid"; then is_simulator=1; fi
  if [[ "$is_simulator" == "1" ]]; then
    build_args+=(-destination "platform=iOS Simulator,id=$udid")
    products_dir="$derived_data/Build/Products/$configuration-iphonesimulator"
  else
    build_args+=(-destination "platform=iOS,id=$udid")
    products_dir="$derived_data/Build/Products/$configuration-iphoneos"
    if [[ -n "$team" ]]; then
      build_args+=(-allowProvisioningUpdates "DEVELOPMENT_TEAM=$team")
    fi
  fi

  if [[ "$mode" != "install-only" ]]; then
    build_start=$(date +%s)
    echo "building $scheme ($configuration) for $udid..." >&2
    # An incremental build with nothing to do leaves the .app unwritten, which
    # the freshness guard cannot tell from a stale one. Removing the bundles
    # makes every build write its .app, at the cost of re-running the link
    # and copy steps only.
    rm -rf "$products_dir"/*.app

    # No `-quiet`: it hides the compiler and signing errors a failed build
    # needs to show.
    if ! xcodebuild "${build_args[@]}" build 1>&2; then
      if [[ "$is_simulator" == "0" ]]; then
        echo "error: xcodebuild failed for physical device $udid. If the errors above" >&2
        echo "       are about signing or provisioning, set DEVELOPMENT_TEAM to your Apple" >&2
        echo "       team ID (install_env or the shell). Then the build signs with" >&2
        echo "       -allowProvisioningUpdates." >&2
      fi
      return 1
    fi
  else
    echo "install-only: reusing prior build for $udid" >&2
  fi

  app=$(golem_pick_app "$products_dir")
  if [[ -z "$app" ]]; then
    echo "error: no .app bundle found in $products_dir (build may have been skipped — re-run without install-only)" >&2
    return 1
  fi
  if [[ "$mode" != "install-only" ]]; then
    golem_require_fresh "$app" "$build_start" || return 1
  fi

  golem_ios_install_app "$udid" "$app"
}
