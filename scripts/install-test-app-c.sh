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
  )
fi

exec bash scripts/install-app-c.sh "$@"
