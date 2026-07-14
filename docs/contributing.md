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
| Test app (`test-app` / `test-app-b` / `test-app-e`) | only if app has own tests | ✓ | — |
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

## Release notes

Release notes are generated from PRs by [`scripts/release-notes.sh`](../scripts/release-notes.sh) (run by `release.sh` at release time), so each PR describes its own user-facing changes in a marked block in the description — one line per change, typed:

```
<!-- release-notes -->
- feat: swipe_until scrolls until a target is visible
- fix: scroll overshoot no longer reverses on RTL
- breaking: --platform is now required when both companions are embedded
<!-- /release-notes -->
```

A PR may list several lines and mix types; each is bucketed under **Breaking / Features / Fixes**. The [PR template](../.github/pull_request_template.md) pre-fills the block, and a [Release note check](../.github/workflows/release-note-check.yml) fails any PR that omits it — unless the PR is labelled `no-release-note` (ci/docs/chore-only) or opened by a bot. Dependency bumps need **no** line: the notes compute those from the lockfile diff (direct deps only; transitive churn collapses to a count).

## Developer Certificate of Origin

golem is [source-available under FSL-1.1](../LICENSE) (Apache-2.0 future license). To keep the project's right to ship and eventually relicense under Apache-2.0, contributions are accepted under the [Developer Certificate of Origin](https://developercertificate.org/) — a lightweight sign-off (no CLA paperwork).

Sign off every commit with `git commit -s`, which appends a line taken from your git identity:

```
Signed-off-by: Your Name <you@example.com>
```

By signing off you certify the DCO 1.1: that you wrote the contribution or have the right to submit it under the project's license, and that you understand it is public and recorded.

**Keeping your email private.** The sign-off needs a stable, attributable identity — not necessarily a personal address. Use GitHub's private no-reply email (`ID+username@users.noreply.github.com`, from GitHub → Settings → Emails → *Keep my email addresses private*) as your `git config user.email`; `-s` will use it and your real address stays out of the history.

**Forgot to sign off?** It's fixable — add it retroactively and force-push your branch:

```bash
git commit --amend --signoff          # the most recent commit
git rebase --signoff origin/main      # every commit since main
```

A [DCO status check](../.github/workflows/dco.yml) runs on every pull request and **fails** if any non-merge commit lacks a `Signed-off-by` matching its author — so a missing sign-off is caught before merge, never silently accepted.

