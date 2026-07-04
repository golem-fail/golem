# Golem

*A golem of clay needs an inscribed word to move. Yours is TOML.*

**Cross-platform mobile UI testing.** Mold your tests once in TOML, animate them on iOS and Android — the same flow drives a simulator, an emulator, or a physical device.

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

**Prerequisites:**

- **A Rust toolchain** — only to build golem from source, which is the only distribution path today (no prebuilt binaries are published yet).
- **Platform toolchains** — building from source also compiles the on-device [companions](docs/companions.md): Xcode for the iOS companion, the Android SDK + Gradle for the Android one. A missing toolchain is skipped with a warning rather than failing the build — you just won't be able to target that platform until it's present.
- **At runtime**, driving a platform needs its CLI tools (`xcrun simctl` for iOS, `adb` for Android) plus a booted simulator/emulator or a connected device.

```bash
# Build + install the `golem` binary from source
cargo install --path golem-cli

# Scaffold a project (golem.toml, flows/, __fixtures__/, __mixins__/, .golem/)
golem init

# Create a flow template at flows/login.test.toml
golem create login

# (optional) scaffold an install script for your app — native-ios, native-android, or tauri
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
| [CLI Reference](docs/cli-reference.md) | Every command and flag — `run`, `tree`, `devices`, `init`, `create`, `install-script`. |
| [Test Structure](docs/test-structure.md) | Flow anatomy: blocks, steps, selectors, coverage strategies, subflows, data-driven tests, variables, fake-data generators, multi-app flows. |
| [Selectors](docs/selectors.md) | The full selector reference: text/label/index/state, traits, relational + geometric `contains`/`inside`, nesting, and resolution order. |
| [Actions Reference](docs/actions-reference.md) | The complete action vocabulary, grouped by category. |
| [Accessibility](docs/accessibility.md) | The automatic a11y audit: levels, checks, thresholds, confidence, and how to read the annotated screenshot. |
| [App Install](docs/app-install.md) | Install scripts, the install cache, `--rebuild` / `--no-build`, supported frameworks. |
| [Output Formats](docs/output-formats.md) | `human`, `json`, `junit`, `toon`. |
| [Error Codes](docs/error-codes.md) | The `EF408`-style code system and full registry. |
| [Unsupported](docs/unsupported.md) | Known limitations. |

**Contributing & internals:** [Architecture](docs/architecture.md) · [Companions](docs/companions.md) · [Contributing](docs/contributing.md) · [Versioning](docs/versioning.md)

## Contributing

See [Contributing](docs/contributing.md) for the build, test, and e2e workflow. In short: `cargo t` for unit tests (nextest), `cargo clippy --workspace --all-targets` to lint, and run an e2e flow live on a sim/emulator per the change matrix.
