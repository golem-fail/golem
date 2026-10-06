# test-app-d — Flutter counter

Minimal Flutter app that exercises golem's **flutter install-script** template
(`golem-cli/templates/install-scripts/flutter.sh`).

- **Bundle id:** `fail.golem.testd` (Android `applicationId` in
  `android/app/build.gradle.kts`, iOS `PRODUCT_BUNDLE_IDENTIFIER`). Keep it in
  sync with `golem.toml`.
- **UI:** a `Flutter Counter` heading, a count below it, and `+` / `-`
  buttons, the same as the other test apps, so cross-app flows drive the same
  way. The heading carries `Semantics(identifier: 'counter-title')`.
- **Host needs:** the Flutter SDK (3.19 or later).

## Build

```
# from the repo root, via golem:
cargo run -- run e2e/flutter_lifecycle.test.toml --platform ios|android
```

`android/` and `ios/` are committed, as in a real Flutter app. Flutter's own
`.gitignore` files leave out what `flutter build` regenerates.
