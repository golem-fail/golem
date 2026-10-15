### tauri
<!-- summary: tauri ios/android build, then install -->

`tauri ios build` / `tauri android build` then install. Detects package manager from lockfiles (`npm` / `yarn` / `pnpm` / `bun` / `cargo tauri`). Reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release`. A release build on Android needs a `signingConfig` in `src-tauri/gen/android`, because Android installs only signed APKs. Without one, the script stops with an error that names the signing problem.
- `TAURI_BUILD_CONFIG`: passed to `tauri … build --config`. It is JSON, or the path to a config file that tauri merges over `tauri.conf.json`.
- `TAURI_BUILD_FEATURES`: passed to `tauri … build --features`, comma-separated.

The script installs only an artifact that the current build wrote. It fails if the `.app` or the APK is older than the build start.
