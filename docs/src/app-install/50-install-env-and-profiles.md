## Parameterizing installs: `install_env` + profiles
<!-- summary: install_env values and --profile app definitions -->

Two composable knobs let one script cover staging/sandbox/prod, Debug/Release, or local-vs-cloud builds without hand-editing scripts.

### `install_env` — inject environment values

Declare env vars on an app; golem injects them into the install script's process:

```toml
[[apps]]
name = "app"
bundle = "com.example.app"
install_script = "scripts/install.sh"
install_env = { APP_ENV = "staging", SANDBOX_ID = "${sandbox_id}" }
```

Values interpolate `${var}` through the var engine **at install time**, so dynamic values flow from `--var`:

```bash
golem run flows/ --var sandbox_id=12345      # SANDBOX_ID becomes "12345"
```

Injection is the existing `--var` — there is no separate `--install-env` flag. Because install runs *before* any flow step, only install-time scopes resolve: CLI `--var`, project/flow vars, and the builtins `_platform` / `_udid` / `_app`. A `${...}` referencing a flow-step, device-, or per-row `each` var **fails the install loudly** (naming the var) rather than silently interpolating to empty and building the wrong artifact.

### Profiles — select app *definitions* by `--profile`

The same app `name` may appear in multiple `[[apps]]` / `[[flow.apps]]` entries, disambiguated by a `profile` field. A `profile`-less entry is the catch-all default. `golem run --profile <name>` picks, per app, the matching entry — else falls back to the catch-all.

```toml
# Default (bare `golem run`): local dev build.
[[apps]]
name = "app-e"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"

# `golem run --profile eas`: same script, cloud build (via install_env).
[[apps]]
name = "app-e"
profile = "eas"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"
install_env = { EXPO_BUILD_MODE = "eas", EAS_PROFILE = "preview" }
```

- Entries key by `(name, profile)`. A flow entry field-merges over the project entry of the **same** key (flow wins); different profile keys never bleed into each other. Profile entries are self-contained — restate shared fields (e.g. `bundle`), nothing inherits across profiles.
- Falling back to the catch-all is by design — golem notes it, it is not an error. It **hard-errors** only when a referenced app has neither a `(name, profile)` entry nor a catch-all. A `--profile` that matches nothing across every app is flagged as a likely typo.
- The script never sees the profile name — `--profile` only selects which entry (hence which `install_script` / `install_env`) is active. This is the CI/local lever: a `ci` profile can use a release/standalone build while the default uses a dev build, without duplicating flows.
