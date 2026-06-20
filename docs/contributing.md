# Contributing

← [Back to README](../README.md) · See also [Architecture](architecture.md) · [Companions](companions.md) · [Versioning](versioning.md)

golem is a Cargo workspace. The agent-facing workflow lives in [`AGENTS.md`](../AGENTS.md) (`CLAUDE.md` symlinks to it); this doc is the human-readable version of the same gates and matrix.

## Gate before every commit

Run these after coding, before committing:

- **Unit tests:** `cargo t` (nextest, debug). **Not** `cargo test --release` — nextest debug is far faster. Output shows only fail/retry/slow by default; add `--status-level pass` for full output.
- **Lint:** `cargo clippy --workspace --all-targets`. The workspace denies `unwrap_used` — no `.unwrap()` in non-test code.
- **iOS companion Swift changes** (`companions/ios`): `./scripts/test-ios-companion.sh` (Swift Testing on a simulator). `cargo t` does **not** cover Swift.
- **E2E** per the matrix below, run live on a simulator/emulator.

New features SHALL add or amend Rust tests — the goal is full unit + e2e coverage. A new test that takes >2s shows up as a nextest SLOW: justify it, or find a faster test with the same coverage.

## What to run, by change type

| Change | unit + clippy | e2e | version bump |
|---|---|---|---|
| Docs / non-code only | skip | skip | — |
| Rust tests only (no non-test code touched) | ✓ | skip | — |
| `*.test.toml` only | skip | that flow, both platforms | — |
| Test app (`test-app` / `test-app-b`) | only if app has own tests | ✓ | — |
| Rendering only (`golem-report`, not run logic) | ✓ | any 1 platform | — |
| Core / functional (runner, driver, element, cli…) | ✓ | 1 android + 1 ios | — |
| One companion only | ✓ | that 1 platform | ✓ |
| Both companions / companion + core | ✓ | both | ✓ |

Any non-test code change — even making a `fn` `pub` — counts as its real category, not "tests only". Prefer an e2e flow relevant to the change; otherwise run a generic flow such as `e2e/cross/tap.test.toml`.

## Running e2e

```bash
# Fast (no companion change): reuses the installed app + companion
cargo run -- run e2e/cross/<flow>.test.toml --no-build --platform android|ios

# Companion changed: bump first (invalidates the install cache), then full build/install
./scripts/bump-version.sh --patch
cargo run -- run e2e/cross/<flow>.test.toml --platform android|ios
```

Versions only ever go forward — a final commit may skip numbers. See [Versioning](versioning.md) for which files carry a version and why a companion change requires a bump (it invalidates the install cache so the new companion actually ships to the device).

## When an e2e fails

Investigate one thing at a time. Identify the cause, summarise it, and check whether it's a known issue. A regression must be fixed before commit unless it's explicitly deferred. Don't add test workarounds that hide an engine bug — fix the engine.

## Where things live

See [Architecture](architecture.md) for the crate map and the end-to-end execution model, and [Companions](companions.md) for the on-device iOS/Android harnesses.
