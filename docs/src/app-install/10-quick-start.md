## Quick start
<!-- summary: golem install-script, and --print / --template without a terminal -->

```bash
golem install-script      # interactive: choose framework, answer prompts
```

The scaffold writes `scripts/install-<app>-<platform>.sh` (for native iOS/Android) or `scripts/install-<app>.sh` (for cross-platform frameworks like Tauri and Expo) and can auto-update `golem.toml`.

**Without a terminal** (for example from an agent), use one of these:

```bash
golem install-script --list           # the frameworks that have a template
golem install-script --print expo     # write the Expo template to stdout, then fill in its {{VALUES}}
golem install-script --template expo --app requester --path apps/requester-app
```

With `--template`, the command asks nothing. Each value that no flag gives is the default its prompt would show, detected from the project as the prompt detects it. golem adds the script to the app's `[[apps]]` entry in `golem.toml`, if there is one. `--app` is the app's name in `[[apps]]` (default `app`). `--path` is the app's project directory. For `native-ios`, `--path` is the `.xcodeproj` or `.xcworkspace`, or a directory that has one. The templates are in [Frameworks the scaffold supports](#frameworks-the-scaffold-supports).

**Monorepos:** a repository can have many apps. Run the command once for each app, with its own `--app` and `--path`, or loop over the `[[apps]]` of `golem.toml`:

```bash
golem install-script --template expo --app requester --path apps/requester-app
golem install-script --template expo --app provider  --path apps/provider-app
```
