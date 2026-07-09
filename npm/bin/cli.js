#!/usr/bin/env node
// Thin launcher: spawn the vendored native `golem` binary, forwarding argv,
// stdio, and the exit code. The native binary is fetched by install.js
// (postinstall). Kept as a JS shim so the published package always has a valid
// `bin` target regardless of postinstall ordering.

'use strict';

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const bin = path.join(__dirname, '..', 'vendor', 'golem');

if (!fs.existsSync(bin)) {
  console.error(
    '@golem-fail/golem: native binary missing — reinstall the package ' +
      '(the postinstall download may have been skipped or blocked).'
  );
  process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
  console.error(`@golem-fail/golem: failed to run golem: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
