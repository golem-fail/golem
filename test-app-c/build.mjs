// The web build: copies src/ into www/, the webDir Capacitor syncs from.
// Deliberately trivial. It exists so the install script's web-build step
// and its freshness check run against a real, separate build output.
import { cpSync, rmSync } from 'node:fs';

rmSync('www', { recursive: true, force: true });
cpSync('src', 'www', { recursive: true });
