# Golem

*A golem of clay needs an inscribed word to move. Yours is TOML.*

**Cross-platform mobile UI testing.** Mold your tests once in TOML, animate them on iOS and Android — the same flow drives a simulator, an emulator, or a physical device.

[![npm](https://img.shields.io/npm/v/%40golem-fail%2Fgolem?logo=npm)](https://www.npmjs.com/package/@golem-fail/golem)
[![GitHub release](https://img.shields.io/github/v/release/golem-fail/golem?logo=github)](https://github.com/golem-fail/golem/releases/latest)
[![npm downloads](https://img.shields.io/npm/dw/%40golem-fail%2Fgolem?logo=npm)](https://www.npmjs.com/package/@golem-fail/golem)
[![binary downloads](https://img.shields.io/github/downloads/golem-fail/golem/total?logo=github&label=binary%20downloads)](https://github.com/golem-fail/golem/releases)
[![license](https://img.shields.io/badge/license-FSL--1.1-blue)](LICENSE)

```toml
[[block]]
name = "login"
steps = [
  { action = "type", on_text = "Email", input = "${fake:email}" },
  { action = "type", on_text = "Password", input = "secret" },
  { action = "tap", on_text = "Sign In" },
  { action = "assert_visible", on_text = "Dashboard", timeout = 10000 },
]
```

## Why golem

Mobile UI tests tend to rot. Common pain:

- **Selectors that break on every redesign** — golem finds elements by visible text, accessibility label, on-screen position, or computed traits, not brittle XPath or pixel coordinates.
- **A separate test suite per platform** — write one TOML flow, run it on iOS *and* Android. Platform-specific blocks handle the few places they genuinely differ.
- **Coordinate taps that drift across screen sizes** — golem resolves taps to element centres, and auto-scrolls to find off-screen targets.
- **Hand-written test data** — built-in `fake:` generators produce realistic, locale-aware names, addresses, phones, and Luhn-valid cards, seedable for deterministic replay.
- **Flaky setup and teardown** — declarative app install/launch, an install cache that skips redundant rebuilds, and per-step retry/timeout policy.

Tests are plain TOML — readable in review, diffable, no DSL to learn beyond a small vocabulary of [actions](docs/actions-reference.md).

## Quick Start

**Install** — golem ships a self-contained prebuilt binary (the on-device [companions](docs/companions.md) are baked in), so installing needs no Rust, Xcode, or Android SDK:

```bash
brew install golem-fail/golem/golem            # macOS (recommended)
npm install -D @golem-fail/golem               # per-project dev dep (also pnpm/bun/yarn)
curl -fsSL https://raw.githubusercontent.com/golem-fail/golem/main/scripts/install.sh | sh   # fallback
```

See [Installing golem](docs/distribution.md) for every channel, CI usage, and version pinning. Prebuilt for macOS arm64 and Linux x86_64/arm64 (iOS driving is macOS-only; Linux drives Android).

**At runtime**, driving a platform needs its device CLI (`xcrun simctl` for iOS, `adb` for Android) plus an available simulator/emulator or a connected device. `golem doctor` checks every prerequisite and prints a copy-paste fix for each miss.

```bash
golem doctor                     # check your device toolchain + environment

# Scaffold a project (golem.toml, flows/, __fixtures__/, __mixins__/, .golem/)
golem init

# Create a flow template at flows/login.test.toml
golem create login

# (optional) scaffold an install script for your app — native-ios, native-android, tauri, or expo
golem install-script

# See connected simulators, emulators, and devices
golem devices

# Run a flow (or a whole directory)
golem run flows/login.test.toml
golem run flows/
```

A minimal flow targets one or more devices and lists steps:

```toml
[flow]
name = "Counter"
tags = ["smoke"]

[[flow.apps]]
name = "app"
bundle = "com.example.myapp"

[[flow.apps.devices]]
os = ["ios:latest", "android:latest"]
type = "phone"

[[block]]
name = "increment"
steps = [
  { action = "tap", on_text = "+" },
  { action = "tap", on_text = "+" },
  { action = "assert_visible", on_text = "2", on_below = "Counter" },
]
```

Inspect the live UI tree of a running app to discover selectors:

```bash
golem tree
```

## What you can do

- **One flow, both platforms** — iOS + Android from the same TOML, with `where = { os = "..." }` blocks for the differences.
- **Rich selectors** — match by text, accessibility label, index, enabled/checked/clickable state, relative position (`on_below`, `on_right_of`, …), and computed traits.
- **Gestures** — tap, double-tap, long-press, swipe, scroll-until-found, pinch, multi-touch, rotation.
- **Assertions & reads** — wait for elements to appear/disappear, check state, read values into variables.
- **Fake data** — `${fake:person}`, `${fake:address}`, `${fake:credit_card}`, and more, locale-aware and seedable.
- **Multi-app flows** — drive several apps in one scenario (e.g. a main app and a companion).
- **Data-driven tests** — run a flow once per data row.
- **Device control** — dark mode, GPS location, permissions, hardware buttons, push notifications (sim/emu).
- **WebView support** — drive web content inside native apps via the Chrome DevTools Protocol.
- **Built-in accessibility audit** — every run automatically checks the visible UI for tiny tap targets, unlabeled controls, low-contrast text, and small text; findings surface in every report, with an annotated screenshot at the `strict` level. On by default, zero config.
- **CI-ready output** — human, JSON, JUnit XML, and a token-optimised `toon` format; every failure carries a [grep-able error code](docs/error-codes.md).

## Documentation

| Doc | What's in it |
|-----|--------------|
| [Installing golem](docs/distribution.md) | Install channels (brew, npm, curl, GitHub Action), runtime prerequisites, `golem doctor`. |
| [CLI Reference](docs/cli-reference.md) | Every command and flag — `run`, `tree`, `devices`, `init`, `create`, `install-script`, `doctor`. |
| [Test Structure](docs/test-structure.md) | Flow anatomy: blocks, steps, selectors, coverage strategies, subflows, data-driven tests, variables, fake-data generators, multi-app flows. |
| [Selectors](docs/selectors.md) | The full selector reference: text/label/index/state, traits, relational + geometric `contains`/`inside`, nesting, and resolution order. |
| [Actions Reference](docs/actions-reference.md) | The complete action vocabulary, grouped by category. |
| [Accessibility](docs/accessibility.md) | The automatic a11y audit: levels, checks, thresholds, confidence, and how to read the annotated screenshot. |
| [App Install](docs/app-install.md) | Install scripts, the install cache, `--rebuild` / `--no-build`, `install_env` + `--profile`, supported frameworks. |
| [Output Formats](docs/output-formats.md) | `human`, `json`, `junit`, `toon`. |
| [Error Codes](docs/error-codes.md) | The `EF408`-style code system and full registry. |
| [Unsupported](docs/unsupported.md) | Known limitations. |

**Contributing & internals:** [Architecture](docs/architecture.md) · [Companions](docs/companions.md) · [Contributing](docs/contributing.md) · [Versioning](docs/versioning.md)

## Contributing

Building from source (`cargo install --path golem-cli`) is the **contributor** path — it needs a Rust toolchain plus the platform toolchains to compile the companions (Xcode for iOS, Android SDK + Gradle for Android; a missing one is skipped with a warning). End users should prefer a prebuilt channel above.

See [Contributing](docs/contributing.md) for the build, test, and e2e workflow. In short: `cargo t` for unit tests (nextest), `cargo clippy --workspace --all-targets` to lint, and run an e2e flow live on a sim/emulator per the change matrix.

Contributions are accepted under a [Developer Certificate of Origin](docs/contributing.md#developer-certificate-of-origin) — sign off your commits with `git commit -s`.

## License

[FSL-1.1](LICENSE) — **source-available**: use, modify, and share golem freely for anything except building a competing product, and each release turns into open-source **Apache-2.0 two years** after it ships.

<sub>Full terms in [LICENSE](LICENSE); "source-available", not open source.</sub>
