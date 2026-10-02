# test-app-c — Capacitor counter

Minimal Capacitor 8 app that exercises golem's **Capacitor install-script**
template (`golem-cli/templates/install-scripts/capacitor.sh`). It is the
Capacitor analogue of `test-app` (Tauri), `test-app-b` (native) and
`test-app-e` (Expo).

- **Bundle id:** `fail.golem.testc` (`capacitor.config.json` → `appId`). Keep it
  in sync with `golem.toml`.
- **UI:** a `Counter` heading, a count below it, and Increment / Decrement
  buttons (`aria-label` + visible `+` / `-`), the same as the other test apps,
  so cross-app flows drive the same way.
- **Web build:** `npm run build` copies `src/` into `www/`, the `webDir`. It is
  deliberately trivial: it exists so the template's web-build step and its
  freshness check run against a real build output.
- **iOS `contentInset`:** left at Capacitor's default (`"never"`), so the e2e
  tap also covers WebView coordinate mapping for a page that is not inset
  (#266).

## Build

```
# from the repo root, via golem:
cargo run -- run e2e/capacitor_lifecycle.test.toml --platform ios|android
```

`scripts/install-test-app-c.sh` runs `npx cap add <platform>` when the native
project is missing, then runs `scripts/install-app-c.sh` (the rendered
template). In a real Capacitor app, `android/` and `ios/` are committed source.
Here they are generated and gitignored, together with `www/` and
`node_modules/`.
