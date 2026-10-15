## Script contract
<!-- summary: The arguments: $1 platform, $2 device id, $3 bundle id, $4 mode -->

Golem invokes the script from the project root with three positional args:

```text
script.sh <platform> <device_udid> <bundle_id>
```

- `platform`: `"ios"` or `"android"`
- `device_udid`: simulator UDID / emulator serial / physical device identifier
- `bundle_id`: from `[[apps]]` or `[[flow.apps]]`

Exit 0 = success, golem launches the app. Nonzero = flow fails with the script's stderr captured; subsequent flows using the same `(device, bundle)` pair are skipped.

Stdout is discarded. Stderr is streamed live via the event system and shows up in:

- Human output: `[install <app>]` prefix
- `results.json` / `results.toon` / `results.xml`: under the top-level `installs` list, with success/duration/exit_code/error

Scripts also support a 4th `"install-only"` arg for manual dev-iteration (skip build, reuse previous artifact). Golem currently always passes empty; this is reserved for a future build-once-install-many optimisation.

### Environment

The script inherits golem's environment, plus:

- **`GOLEM_REBUILD`** — `"1"` when the run passed `--rebuild`, else `"0"`. A golem-injected builtin (the `GOLEM_` prefix is reserved for these). Lets a script that owns a remote/build cache force a fresh build; scripts that ignore it are unaffected.
- **Your `install_env`** — any key/values you declare on the app (see below).
