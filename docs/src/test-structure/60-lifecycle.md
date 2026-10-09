## Lifecycle: Setup & Teardown

**There is no `[[setup]]` block.** A flow's setup is implicit and happens automatically before the first block:

1. **build and install** — once per app and device across the suite, cached (see [App Install](app-install.md)).
2. **app_lifecycle** — per flow, at flow start:
   - `reset` (default) — stop every app in `[[flow.apps]]`, then launch the first. Guarantees fresh state.
   - `launch` — launch the first app only if not already running. Preserves state.
   - `manual` — do nothing; the flow (or its parent) owns the app. `--start <block>` forces this.

Any additional setup you need (e.g. creating a user) is just normal steps, or a [mixin](#reuse-subflow-vs-mixin-vs-fixture) if shared.

**Subflows** never re-build or re-install (that layer isn't re-entered for a `run_flow` child), but the child **does** re-run `app_lifecycle` with *its own* setting — which is why reusable subflows set `app_lifecycle = "manual"` to inherit the parent's running app.

Cleanup after the flow belongs in [Teardown](#teardown).
