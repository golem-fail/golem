# test-app-e — Expo / React Native counter

Minimal Expo app that exercises golem's **Expo install-script** template
(`golem-cli/templates/install-scripts/expo.sh`). It's the Expo analogue of
`test-app` (Tauri) and `test-app-b` (native).

- **Bundle id:** `fail.golem.teste` (`app.json` → `ios.bundleIdentifier` /
  `android.package`). This is authoritative — keep it in sync with `golem.toml`.
- **UI:** a `Counter` heading, a count value below it, and Increment / Decrement
  buttons (`accessibilityLabel` + visible `+` / `-`), matching the other test
  apps so cross-app flows drive identically.

## Local build (what CI verifies)

The install scripts run `expo prebuild` + a **Release** native build (embeds the
JS bundle, so it runs offline with no Metro) and install to a sim/emulator. No
Expo account or signing account needed.

```
# from the repo root, via golem:
cargo run -- run e2e/expo_lifecycle.test.toml --platform ios|android
```

Generated native dirs (`ios/`, `android/`), `node_modules/`, and `.expo/` are
gitignored — `expo prebuild` regenerates the native projects on demand.

## EAS cloud build (unverified)

The `expo.sh` template also supports EAS cloud builds (`EXPO_BUILD_MODE=eas`,
selected via `--profile eas`). That path needs an Expo account (`EXPO_TOKEN`) and
is **not** exercised in golem's CI — validate it against a real Expo project
before relying on it. `eas.json` carries a `preview` profile (simulator builds)
for that path.
