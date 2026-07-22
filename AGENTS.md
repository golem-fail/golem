# golem — agent workflow

`CLAUDE.md` symlinks here. CLI: `README.md` · versions: `docs/versioning.md` · tracking: [GitHub Issues](https://github.com/golem-fail/golem/issues) (preferred) + `docs/roadmap.md` (being migrated — see [Tracking](#tracking-issues-vs-roadmap)).

## Tracking (issues vs roadmap)
Prefer **GitHub Issues** for any work with a clear problem, reproduction, and acceptance criteria — set the issue **Type** (Bug/Feature/Task), scoped **labels** (`platform:`/`host:`/`lang:`/`framework:`/`area:`/`action:`), **Effort**, and **blocked by** where relevant. `docs/roadmap.md` is a **temporary** home for still-vague items (no crisp repro/acceptance); we're gradually migrating them to issues and roadmap.md will eventually be deleted. Rule of thumb: if you can write Problem + Reproduction + Acceptance, file an issue, not a roadmap entry.

## Gate before every commit (after coding)
- Unit: `cargo t` (nextest; NOT `cargo test --release`). Output shows only fail/retry/slow by default (no tailing needed); `--status-level pass` for full output.
- iOS companion Swift logic changed (`companions/ios`): `./scripts/test-ios-companion.sh` (Swift Testing on a sim; `cargo t` does NOT cover Swift — not part of nextest).
- Lint: `cargo clippy --workspace --all-targets` (workspace denies `unwrap_used`).
- Format: `cargo fmt --all -- --check` (uses the pinned toolchain; matches CI). Opt-in pre-push hook: `git config core.hooksPath .githooks`.
- E2E per matrix, live on sim/emu.
- New features SHALL add/amend Rust tests. Goal = full unit + e2e coverage.
- New test >2s = nextest SLOW: justify, or find faster test with same coverage.
- Sign off every commit: `git commit -s` (the DCO check fails unsigned commits — see `docs/contributing.md`). Author identity is the repo-local GitHub no-reply (auto); do NOT override it or reintroduce a personal/work email.
- PR body needs a release-notes block — a `## Release notes` header, then a `category: line` (`fixed:`/`added:`/`internal:`/…) inside the `<!-- release-notes -->` markers (keep the header for human reviewers; it's in the PR template) — or the `no-release-note` label, else the required gate blocks merge.

## Where information belongs (How / What / Why / Why not)
Put each fact where it lives; don't write it in the wrong place.
- **Code = How** — mechanics. Code already states how; don't paraphrase it in comments (skip obvious what/how narration).
- **Tests = What** — spec + expected behavior, via test names + assertions.
- **Commit logs = Why** — why the change was made (background, context, intent). Rationale like "because it became X", "replaced Y", "previously it was Z" goes here, NOT in code comments (rots into a lie after later edits).
- **Code comments = Why not** — why an alternative was rejected, non-obvious constraints, pitfalls avoided. Limit comments to "deliberately did it this way" reasons the code alone can't convey.

Before adding a comment ask: "is this *why not*, or just *why* (excuse for a change)?" — if the latter, don't write it. Match surrounding comment density + style; don't annotate self-explanatory code when neighbors have none.

## Matrix — by change type
| Change | unit+clippy | e2e | bump |
|---|---|---|---|
| Docs / non-code only | skip | skip | — |
| Rust tests only (no non-test code touched) | ✓ | skip | — |
| `*.test.toml` only | skip | that flow, both platforms | — |
| Test app (`test-app`/`test-app-b`/`test-app-e`) | only if app has own tests (none yet) | ✓ | — |
| Rendering only (`golem-report`, not run logic) | ✓ | any 1 platform | — |
| Core / functional (runner, driver, element, cli…) | ✓ | 1 android + 1 ios | — |
| One companion only | ✓ | that 1 platform | ✓ |
| Both companions / companion+core | ✓ | both | ✓ |

Any non-test code change (even a fn → `pub`) = its real category, not "tests only".
Prefer e2e relevant to the change; else any generic flow (`e2e/tap.test.toml`).

## Core invariant: visible tree judges, full tree only hints
golem tests like a human — target/assert ONLY against the visible (filtered) tree (`filter_viewport` on `effective_bounds`/`visible_bounds`, ancestor-clip-aware via IntersectionObserver in webviews). The full (unfiltered) tree is for hints that speed/steer but never decide pass/fail: auto-scroll direction, overshoot reversal, settle fingerprints. Reading the full tree to judge a step succeeded = bug (passes on what the user can't see). See [Visibility model](docs/architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints).

## Run e2e
```
# fast (no companion change): reuses installed app+companion
cargo run -- run e2e/<flow>.test.toml --no-build --platform android|ios
# companion changed: bump first (invalidates cache), then full build/install
./scripts/bump-version.sh --patch
cargo run -- run e2e/<flow>.test.toml --platform android|ios
```
Versions never go backwards; final commit may skip numbers.

## E2E failure
Spawn subagent → report cause/summary → check [GitHub Issues](https://github.com/golem-fail/golem/issues) + `docs/roadmap.md` (known?).
Regression = fix before commit unless user says track it. When done, close the issue (or delete the roadmap entry).
