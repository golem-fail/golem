# setup-golem

A composite GitHub Action that installs the [golem](https://github.com/golem-fail/golem)
CLI on a runner and verifies the environment. Runs on **your** runners, on your
Actions budget.

```yaml
- uses: golem-fail/golem/setup-golem@v0.8.0
  with:
    version: "0.8.0"   # optional; default = latest release
    doctor: "true"     # optional; run `golem doctor` as an environment gate
```

The `@ref` is pinned to a release tag — bump it (e.g. `@v0.9.0`) when you upgrade.

Installs a prebuilt, self-contained `golem` binary (companions baked in) into
`~/.golem/bin` and adds it to `PATH`, then optionally runs `golem doctor`.

Driving devices still needs a device toolchain + a booted device — pair this
with an emulator/simulator provider:

```yaml
# Android
- uses: golem-fail/golem/setup-golem@v0.8.0
  with: { doctor: "false" }        # defer the gate until the emulator is up
- uses: reactivecircus/android-emulator-runner@v2
  with:
    api-level: 34
    script: golem doctor && golem run e2e/flow.test.toml --platform android
```

> **Emulator speed needs KVM.** `android-emulator-runner` runs an x86_64 AVD that
> is only usable at speed with hardware acceleration. GitHub's standard
> `ubuntu-latest` (and macOS) runners provide KVM; older/smaller Linux classes and
> many self-hosted/containerised runners do **not** (a container needs
> `--privileged` or `/dev/kvm` passthrough). On **aarch64 Linux** there is no
> first-class Android emulator — drive `adb`-connected physical/remote devices
> instead. `adb` itself works on every arch.

```yaml
# iOS — on a macOS runner with a preinstalled simulator
- uses: golem-fail/golem/setup-golem@v0.8.0
- run: |
    xcrun simctl boot 'iPhone 16'
    golem run e2e/flow.test.toml --platform ios
```

Prebuilt for **macOS arm64** and **Linux x86_64/arm64** (static musl). iOS driving
is macOS-only; on a Linux runner golem defaults to `--platform android` (an
explicit `--platform ios` is honoured but fails loudly). See
[Installing golem](https://github.com/golem-fail/golem/blob/main/docs/distribution.md)
for the full channel + runtime matrix.
