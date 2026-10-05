#!/usr/bin/env bash
# Install script for test-app-c (Capacitor), as golem.toml registers it.
#
# A real Capacitor app commits its android/ and ios/ projects, and the
# template (scripts/install-app-c.sh, rendered from capacitor.sh) refuses to
# create them. This repo does not commit generated projects, so this wrapper
# runs `cap add` for a missing platform first, then hands over to the
# rendered template unchanged.
#
# Same arguments as the template: platform, device id, bundle id, [mode].

set -euo pipefail

PLATFORM="${1:?platform required}"
APP_DIR="test-app-c"

if [[ ! -d "$APP_DIR/$PLATFORM" ]]; then
  echo "install-test-app-c: generating the $PLATFORM project..." >&2
  (
    cd "$APP_DIR"
    if [[ ! -d node_modules ]]; then npm install 1>&2; fi
    # `cap add` copies webDir into the new project, so it must exist.
    npm run build 1>&2
    npx cap add "$PLATFORM" 1>&2
    # The app icon: `cap add` writes Capacitor's default, and the generated
    # project is not committed, so the golem icon is applied to each new one.
    # The generator adds its own adaptive-icon inset, so the full master is
    # the foreground.
    mkdir -p build/icon-assets
    cp icon-1024.png build/icon-assets/icon-only.png
    cp icon-1024.png build/icon-assets/icon-foreground.png
    cp icon-background.png build/icon-assets/icon-background.png
    npx capacitor-assets generate "--$PLATFORM" --assetPath build/icon-assets \
      --iconBackgroundColor '#ffffff' --iconBackgroundColorDark '#ffffff' 1>&2
  )
fi

exec bash scripts/install-app-c.sh "$@"
