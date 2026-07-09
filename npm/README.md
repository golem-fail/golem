# @golem-fail/golem

Prebuilt-binary wrapper for [golem](https://github.com/golem-fail/golem), a
mobile UI testing framework. Install it as a **per-project dev dependency** so
the version is pinned in your lockfile — which dovetails with golem's
host↔companion version lock for reproducible local + CI runs.

```sh
npm install -D @golem-fail/golem
# or
pnpm add -D @golem-fail/golem
bun add -d @golem-fail/golem
yarn add -D @golem-fail/golem
```

Then run it via `npx golem` (or a `package.json` script):

```sh
npx golem doctor
npx golem run e2e/flow.test.toml
```

`postinstall` downloads a self-contained `golem` binary (iOS/Android companions
baked in) matched to your platform and verifies its sha256. Driving devices
still needs host tooling (`adb`, Xcode/simulators) — `golem doctor` checks it.

Supported: macOS arm64, and Linux x86_64 + arm64 (static musl). iOS is
macOS-only — Linux drives Android. The pinned version is downloaded by default;
set `GOLEM_VERSION` to override.

## pnpm / bun

pnpm (v10+) and bun don't run dependency install scripts by default, so the
`postinstall` download won't fire until you allow this package:

- **pnpm** — add to your root `package.json`:
  ```json
  { "pnpm": { "onlyBuiltDependencies": ["@golem-fail/golem"] } }
  ```
  (or run `pnpm approve-builds`).
- **bun** — add to your root `package.json`:
  ```json
  { "trustedDependencies": ["@golem-fail/golem"] }
  ```

npm and Yarn run it by default. If a download was skipped, `npx golem` prints a
clear "native binary missing — reinstall" error rather than failing obscurely.

