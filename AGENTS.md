# golem — agent workflow

`CLAUDE.md` symlinks here. CLI: `README.md` · versions: `docs/versioning.md` · todo: `docs/roadmap.md`.

## Gate before every commit (after coding)
- Unit: `cargo t` (nextest; NOT `cargo test --release`).
- Lint: `cargo clippy --workspace --all-targets` (workspace denies `unwrap_used`).
- E2E per matrix, live on sim/emu.
- New features SHALL add/amend Rust tests. Goal = full unit + e2e coverage.
- New test >2s = nextest SLOW: justify, or find faster test with same coverage.

## Matrix — by change type
| Change | unit+clippy | e2e | bump |
|---|---|---|---|
| Docs / non-code only | skip | skip | — |
| Rust tests only (no non-test code touched) | ✓ | skip | — |
| `*.test.toml` only | skip | that flow, both platforms | — |
| Test app (`test-app`/`test-app-b`) | only if app has own tests (none yet) | ✓ | — |
| Rendering only (`golem-report`, not run logic) | ✓ | any 1 platform | — |
| Core / functional (runner, driver, element, cli…) | ✓ | 1 android + 1 ios | — |
| One companion only | ✓ | that 1 platform | ✓ |
| Both companions / companion+core | ✓ | both | ✓ |

Any non-test code change (even a fn → `pub`) = its real category, not "tests only".
Prefer e2e relevant to the change; else any generic flow (`e2e/cross/tap.test.toml`).

## Run e2e
```
# fast (no companion change): reuses installed app+companion
cargo run -- run e2e/cross/<flow>.test.toml --no-build --platform android|ios
# companion changed: bump first (invalidates cache), then full build/install
./scripts/bump-version.sh --patch
cargo run -- run e2e/cross/<flow>.test.toml --platform android|ios
```
Versions never go backwards; final commit may skip numbers.

## E2E failure
Spawn subagent → report cause/summary → check `docs/roadmap.md` (known?).
Regression = fix before commit unless user says roadmap it. Delete roadmap entry when done.
