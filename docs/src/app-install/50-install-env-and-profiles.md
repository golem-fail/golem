## Parameterizing installs: `install_env` + profiles
<!-- summary: install_env values and --profile app definitions -->

One script can cover staging and production, Debug and Release, or a local and a cloud build. Two settings do this: `install_env` and profiles.

### `install_env` — inject environment values

golem sets these variables in the install script's environment:

```toml
[[apps]]
name = "app"
bundle = "com.example.app"
install_script = "scripts/install.sh"
install_env = { APP_ENV = "staging", SANDBOX_ID = "${sandbox_id}" }
```

`${var}` resolves at install time, so `golem run --var sandbox_id=12345` sets `SANDBOX_ID` to `12345`. The install runs before any step, so only these resolve: `--var`, project and flow vars, and the builtins `_platform`, `_udid` and `_app`. Any other variable fails the install, and the error names it.

### Profiles — select app *definitions* by `--profile`

Several `[[apps]]` or `[[flow.apps]]` entries can share a `name` if each has its own `profile`. An entry without `profile` is the default. `golem run --profile <name>` uses, for each app, the entry with that profile, else the default.

```toml
# golem run: local build.
[[apps]]
name = "app-e"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"

# golem run --profile eas: the same script, a cloud build.
[[apps]]
name = "app-e"
profile = "eas"
bundle = "com.example.teste"
install_script = "scripts/install-app-e.sh"
install_env = { EXPO_BUILD_MODE = "eas", EAS_PROFILE = "preview" }
```

- A flow entry merges over the project entry with the same name and profile; the flow wins. Entries of different profiles share nothing, so repeat fields such as `bundle`.
- The default as a fallback is not an error. It is an error only when an app has neither its profile's entry nor a default. A `--profile` that no app has gets a warning: it is likely a typo.
- The script does not see the profile. The profile only picks the entry, and so its `install_script` and `install_env`.
