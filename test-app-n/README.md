# test-app-n — NativeScript counter

Minimal NativeScript 9 app (plain JavaScript, XML UI) that exercises golem's
**nativescript install-script** template
(`golem-cli/templates/install-scripts/nativescript.sh`).

- **Bundle id:** `fail.golem.testn` (`nativescript.config.ts` → `id`). Keep it
  in sync with `golem.toml`.
- **UI:** a `Counter` label, a count below it, and `+` / `-` buttons, the
  same as the other test apps, so cross-app flows drive the same way.
- **CLI:** `nativescript` is a devDependency, so the install script runs
  `npx ns` and needs no global CLI.

## Build

```
# from the repo root, via golem:
cargo run -- run e2e/nativescript_lifecycle.test.toml --platform ios|android
```

`ns build` generates `platforms/` on demand. It is gitignored, together with
`node_modules/` and `hooks/`.
