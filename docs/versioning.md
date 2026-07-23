# Versioning

← [Back to README](../README.md) · See also [Companions](companions.md) · [Contributing](contributing.md)

All golem components share a single version number.

Prefer the bump script for simplicity:

```bash
./scripts/bump-version.sh --patch   # read current version, bump X.Y.Z -> X.Y.Z+1
./scripts/bump-version.sh 0.5.0      # or set an explicit version
```

It updates all locations listed below and verifies the result. The manual list is kept here for reference.

## Releasing

A release is cut by **pushing a `vX.Y.Z` tag** — the `Release` workflow (`.github/workflows/release.yml`) does everything else. **Do not run `scripts/release.sh` by hand:** the workflow is live (gated on the `RELEASE_ENABLED` repo variable, currently `true`) and invokes `release.sh` itself from the tag; running it locally too would double-publish.

1. **Bump** on a branch — `./scripts/bump-version.sh --patch` (or an explicit `X.Y.Z`) — then open a PR and merge it. The bump edits the companions, so CI runs `android-tests`/`ios-tests` (they're path-gated and skipped on pure-Rust PRs); that's expected and fast (`android-tests` is compile-only).
2. **Tag the merged commit** and push:

   ```bash
   git tag -a vX.Y.Z <merge-sha> -m "golem vX.Y.Z"
   git push origin vX.Y.Z
   ```

3. The **Release workflow** then, from that tag: builds the macOS binary (both companions embedded), **creates the GitHub Release** + uploads the mac asset + pushes the Homebrew tap formula; builds + uploads the Linux (x86_64 + aarch64) assets; and **publishes `@golem-fail/golem` to npm**.

Notes:

- **npm publish is non-idempotent** — a version can't be re-published. If a release is botched, bump to the next patch rather than retrying the same tag.
- Release notes are generated automatically (diffed against the previous tag), aggregating the `## Release notes` blocks from the merged PRs.
- Check the workflow is enabled with `gh variable get RELEASE_ENABLED`; `gh variable set RELEASE_ENABLED --body true` enables it, any other value disables it (see the `release.yml` header comment).

## Automatic (workspace-inherited)

Set once in root `Cargo.toml` under `[workspace.package]`:

```toml
[workspace.package]
version = "X.Y.Z"
```

These crates inherit via `version.workspace = true` — no changes needed:

- golem-cli
- golem-runner
- golem-parser
- golem-driver
- golem-element
- golem-devices
- golem-vars
- golem-report
- golem-email

## Manual updates required

### Test app

| File | Field |
|------|-------|
| `test-app/src-tauri/Cargo.toml` | `version` |
| `test-app/package.json` | `version` |
| `test-app/src-tauri/tauri.conf.json` | `version` |
| `test-app/src-tauri/gen/apple/golem-test-app_iOS/Info.plist` | `CFBundleShortVersionString`, `CFBundleVersion` |

### Companion apps (health endpoint)

| File | Location |
|------|----------|
| `companions/ios/GolemRunnerUITests/RequestRouter.swift` | `"version"` in `handleHealth()` |
| `companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServer.java` | `"version"` in `/health` handler |

### Companion test assertions

| File | Location |
|------|----------|
| `companions/ios/GolemRunnerUITests/GolemRunnerUITests.swift` | `"version"` in health check test |
| `companions/android/app/src/androidTest/java/fail/golem/companion/CompanionServerTest.java` | `"version"` in health check test |
