# Installing golem

The `golem` binary is **self-contained** — the on-device
[companions](companions.md) are baked in, so installing golem needs no Rust,
Xcode, or Android SDK. Pick a channel below; they all install the same prebuilt
binary.

> Driving a device still needs a device toolchain (`adb`, or Xcode +
> simulators) and a booted device — things a binary can't carry. After
> installing, run **`golem doctor`** to check your environment (see [below](#golem-doctor)).

**Platform support:** macOS **arm64** only today. Linux (static musl) is planned;
iOS is macOS-only. On an unsupported platform the installers fail with a clear
message rather than a broken install.

## Which channel?

| You are… | Use | Updates via |
|---|---|---|
| A developer on a Mac | **Homebrew** | `brew upgrade` |
| Adding golem to a project / CI | **npm dev-dependency** | bump the dep (pinned in your lockfile) |
| Anywhere, no package manager | **`curl \| sh`** | re-run the one-liner |
| GitHub Actions | **`setup-golem` action** | re-pin the action ref |

Homebrew and the npm dev-dependency are **recommended** — they manage `PATH` and
updates for you. The npm route additionally pins the version in your lockfile,
which dovetails with golem's host↔companion version lock for reproducible CI.

## Homebrew

```sh
brew install golem-fail/golem/golem
# later:
brew upgrade golem
```

The formula prints the runtime prerequisites as a caveat and points you at
`golem doctor`.

## npm / pnpm / bun / yarn

Install as a **per-project dev dependency** so the version is pinned in your
lockfile, then run it with `npx golem` (or a `package.json` script):

```sh
npm install -D @golem-fail/golem
# or: pnpm add -D @golem-fail/golem  ·  bun add -d @golem-fail/golem  ·  yarn add -D @golem-fail/golem

npx golem doctor
npx golem run e2e/flow.test.toml
```

A `postinstall` step downloads the platform-matched binary and verifies its
checksum. **pnpm (v10+) and bun** don't run install scripts by default — allow
this package:

- **pnpm** — `{ "pnpm": { "onlyBuiltDependencies": ["@golem-fail/golem"] } }` in your root `package.json` (or `pnpm approve-builds`).
- **bun** — `{ "trustedDependencies": ["@golem-fail/golem"] }`.

## curl | sh

The sudo-free fallback. Installs to `~/.golem/bin` and prints a `PATH` hint:

```sh
curl -fsSL https://raw.githubusercontent.com/golem-fail/golem/main/scripts/install.sh | sh

# pin a version:
curl -fsSL https://raw.githubusercontent.com/golem-fail/golem/main/scripts/install.sh | GOLEM_VERSION=0.7.0 sh
```

Idempotent — "updating" is just re-running it. Honours `GOLEM_VERSION` (pin) and
`GOLEM_INSTALL_DIR` (default `~/.golem/bin`).

## GitHub Actions

```yaml
- uses: golem-fail/golem/setup-golem@v1
  with:
    version: "0.7.0"   # optional; default = latest
    doctor: "true"     # optional; gate on `golem doctor`
```

Runs on your runners. Pair it with an emulator/simulator provider —
[`reactivecircus/android-emulator-runner`](https://github.com/ReactiveCircus/android-emulator-runner)
for Android, or a macOS runner with a booted simulator for iOS. See
[`setup-golem`](../setup-golem/README.md) for full examples.

## golem doctor

`golem doctor` reports everything golem needs to drive a device, each miss with
a copy-paste fix:

```
$ golem doctor
golem doctor
  ✓ ~/.golem writable — yes
  ✓ adb (Android) — found
  ✓ Android companion — embedded
  ✓ xcrun (iOS) — found
  ✓ simctl (iOS) — found
  ✓ iOS companion — embedded
  ✓ booted device — 1 (1 android, 0 ios)
  ✓ ffmpeg (optional) — found
  ✓ drivable platform — android, ios
```

It exits non-zero when no platform is drivable, so CI can gate on it. A single
missing CLI is a warning as long as the other platform still works. See
[CLI Reference](cli-reference.md#golem-doctor).

## Building from source (contributors)

Building from source is now the **contributor** path, not the install path. It
needs a Rust toolchain plus the platform toolchains to compile the companions
(Xcode for iOS, Android SDK + Gradle for Android):

```sh
cargo install --path golem-cli
```

A missing platform toolchain is skipped with a warning rather than failing the
build — you just can't target that platform until it's present. See
[Contributing](contributing.md) for the full build/test/e2e workflow.
