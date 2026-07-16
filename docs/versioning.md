# Versioning

← [Back to README](../README.md) · See also [Companions](companions.md) · [Contributing](contributing.md)

All golem components share a single version number.

Prefer the bump script for simplicity:

```bash
./scripts/bump-version.sh --patch   # read current version, bump X.Y.Z -> X.Y.Z+1
./scripts/bump-version.sh 0.5.0      # or set an explicit version
```

It updates all locations listed below and verifies the result. The manual list is kept here for reference.

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
