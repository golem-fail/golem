#!/usr/bin/env node
// postinstall: download the arch-matched prebuilt `golem` binary, verify its
// sha256, and extract it into vendor/. The binary is self-contained (companions
// baked in). The version is pinned to this package's version (so the lockfile
// dovetails with the host↔companion lock); GOLEM_VERSION overrides it.
//
// Dependency-free by design: Node built-ins + the system `tar` (present on the
// macOS/Linux targets we ship). GOLEM_BASE_URL points the download at a mirror
// or a local dir laid out as <tag>/<asset> (used for offline tests).

'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { execFileSync } = require('child_process');

const pkg = require('./package.json');

const VERSION = process.env.GOLEM_VERSION || pkg.version;
const BASE_URL =
  process.env.GOLEM_BASE_URL ||
  'https://github.com/golem-fail/golem/releases/download';
const VENDOR = path.join(__dirname, 'vendor');
const BIN = path.join(VENDOR, 'golem');

function fail(msg) {
  console.error(`@golem-fail/golem: ${msg}`);
  process.exit(1);
}

function target() {
  const p = process.platform;
  const a = process.arch;
  if (p === 'darwin' && a === 'arm64') return 'aarch64-apple-darwin';
  if (p === 'linux' && a === 'x64') return 'x86_64-unknown-linux-musl';
  if (p === 'linux' && a === 'arm64') return 'aarch64-unknown-linux-musl';
  // iOS is macOS-only; Linux drives Android. Windows has no prebuilt binary.
  fail(
    `unsupported platform ${p}/${a} — golem ships prebuilt binaries for macOS ` +
      `arm64 and Linux x86_64/arm64 (build from source: cargo install --path golem-cli)`
  );
}

// GET with redirect following, over http or https (local test servers use http).
function get(url, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    const lib = url.startsWith('https:') ? require('https') : require('http');
    lib
      .get(url, (res) => {
        const { statusCode, headers } = res;
        if (statusCode >= 300 && statusCode < 400 && headers.location) {
          if (redirectsLeft === 0) return reject(new Error('too many redirects'));
          res.resume();
          const next = new URL(headers.location, url).toString();
          return resolve(get(next, redirectsLeft - 1));
        }
        if (statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${statusCode} for ${url}`));
        }
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve(Buffer.concat(chunks)));
      })
      .on('error', reject);
  });
}

async function main() {
  if (process.env.GOLEM_SKIP_DOWNLOAD) {
    console.error('@golem-fail/golem: GOLEM_SKIP_DOWNLOAD set — skipping binary download.');
    return;
  }

  const triple = target();
  const asset = `golem-${VERSION}-${triple}.tar.gz`;
  const url = `${BASE_URL}/v${VERSION}/${asset}`;

  console.error(`@golem-fail/golem: downloading ${asset}…`);
  const [tarball, shaFile] = await Promise.all([get(url), get(`${url}.sha256`)]);

  // Verify: the .sha256 payload is "<hex>  <filename>".
  const expected = shaFile.toString('utf8').trim().split(/\s+/)[0];
  const actual = crypto.createHash('sha256').update(tarball).digest('hex');
  if (!expected || expected !== actual) {
    fail(`checksum mismatch for ${asset} (expected ${expected || '<none>'}, got ${actual})`);
  }

  // Extract via system tar (no npm tar dependency).
  fs.mkdirSync(VENDOR, { recursive: true });
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'golem-'));
  const tarPath = path.join(tmp, asset);
  fs.writeFileSync(tarPath, tarball);
  try {
    execFileSync('tar', ['xzf', tarPath, '-C', tmp]);
  } catch (e) {
    fail(`failed to extract ${asset}: ${e.message}`);
  }
  const extracted = path.join(tmp, 'golem');
  if (!fs.existsSync(extracted)) fail(`archive did not contain a 'golem' binary`);
  fs.copyFileSync(extracted, BIN);
  fs.chmodSync(BIN, 0o755);
  fs.rmSync(tmp, { recursive: true, force: true });

  console.error(`@golem-fail/golem: installed golem ${VERSION} → ${BIN}`);
  console.error('Next: run `npx golem doctor` to check your device toolchain.');
}

main().catch((e) => fail(e.message));
