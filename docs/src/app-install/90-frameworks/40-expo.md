### expo
<!-- summary: Expo prebuild and a local Release build, or EAS; golem run --dev -->

Expo / React Native, one script for both platforms. `EXPO_BUILD_MODE` picks the build:

- `local` (default): `expo prebuild`, then a **Release** native build, then `simctl` or `adb` install. The build embeds the JS bundle, so the app runs without Metro. It needs no Expo account and no signing account on a simulator or emulator.
- `eas`: an EAS cloud build, downloaded and installed. It uses the latest finished build unless `GOLEM_REBUILD=1`, and needs `EXPO_TOKEN`. **golem's own CI does not test this mode**, so try it on your project first.

The script finds the package manager from the lockfile, and the iOS scheme from the generated project.

The template is for Expo only: it needs `expo prebuild`. A **bare React Native** project (no `expo` dependency) commits its own `ios/` and `android/`; use **native-ios** and **native-android** for it.

**Iterating on JS: `golem run --dev`.** A Release build embeds the bundle, so each JS change needs a native rebuild. Instead:

1. Build and install a Debug build once: `npx expo run:ios` or `npx expo run:android`.
2. Keep `npx expo start` running.
3. Run `golem run --dev`. golem skips the build and the install. Each flow relaunches the app, which loads the current bundle from the dev server.

If the dev server does not answer, golem fails with `H503`. Use `--dev` for local work only: a CI run should test the artifact that ships. It works on simulators and emulators; a physical device also needs `adb reverse tcp:8081 tcp:8081` or a LAN host that the device can reach.

**Broken JS fails with `A501`, not as a failing test:**

- Before any flow, `--dev` asks the dev server for the bundle. A build error stops the run and quotes it (`SyntaxError: App.tsx: Unexpected token (9:15)`). On iOS this is the only signal, because the error overlay shows nothing golem can read.
- A step that fails while React Native's error overlay shows reports `A501` with the overlay's message. This covers the native redbox (the bundle never ran; Android only, an iOS simulator stays blank) and the JS LogBox (the app threw after it mounted; both platforms, with the source position, `boom at App.tsx (35:43)`).

golem ignores a LogBox that is minimized to its badge, or that shows a console warning or error: the app under it still works.
