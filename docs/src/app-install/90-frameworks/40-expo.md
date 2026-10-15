### expo
<!-- summary: Expo prebuild and a local Release build, or EAS; golem run --dev -->

Expo / React Native. One cross-platform script honouring `EXPO_BUILD_MODE`:
- `local` (default): `expo prebuild` + a **Release** native build (embeds the JS bundle → runs offline, no Metro), then `simctl` / `adb` install. Fully local — no Expo account, works on a simulator/emulator with no signing account.
- `eas`: build in the cloud via EAS, download the artifact, install it. Reuses the latest finished build unless `GOLEM_REBUILD=1`. Needs `EXPO_TOKEN`. **Written but unverified in golem's own CI** (no account) — validate against a real project before relying on it.

Detects package manager from lockfiles; auto-discovers the iOS scheme from the generated app project when not set.

Expo (managed) only — it relies on `expo prebuild` to generate the native projects. **Bare React Native** (a `@react-native-community/cli` project with no `expo` dependency) commits its own `ios/` + `android/` and has no `prebuild`; use **native-ios** + **native-android** for those.

**Iterating on JS: `golem run --dev`.** A Release build embeds the bundle, so
every JS change costs a full native rebuild. Build a Debug build once
(`npx expo run:ios` / `npx expo run:android` — these also install and launch
it), leave `npx expo start` running, and then `golem run --dev` skips
build+install entirely: the per-flow relaunch re-fetches the current bundle
from the dev server, so a JS edit is picked up with no rebuild and no
reinstall. golem waits for the dev server and fails with `H503` if it never
answers, rather than letting the resulting red error screen read as a
missing selector. Local iteration only — a CI run should test the artifact
it ships. Simulators and emulators only; a physical device also needs
`adb reverse tcp:8081 tcp:8081` or a reachable LAN host.

**Broken JS is reported as `A501`, not as a failing test.** With a bundle
that can't build, every selector times out and the run would otherwise
blame the flow. Two things prevent that:

- Before any flow runs, `--dev` asks the dev server for the app's bundle. A
  build failure stops the run and quotes the error (`SyntaxError: App.tsx:
  Unexpected token (9:15)`). This is the only signal available on iOS,
  where React Native's error overlay renders nothing golem can see.
- During a run, a step that fails against a visible error overlay is
  reported as `A501` with the overlay's own message instead of as a missing
  selector. This covers both of React Native's overlays: the native redbox,
  shown when the bundle never ran (Android — the same failures leave an iOS
  simulator blank), and the JS LogBox, shown when the app mounted and then
  threw, which renders on both platforms and also reports the source
  position (`boom at App.tsx (35:43)`).

  A LogBox minimised to its badge, or open on a console warning or console
  error, leaves a working app underneath and is deliberately left alone —
  a flow failing there is failing on its own merits.
