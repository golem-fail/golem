## Secrets
<!-- summary: secret in a fixture or on a save_to step: golem masks the value in all output -->

golem shows variable values in its output: the step lines, the reports, the logs and the MCP results. Tag a value as secret where it enters the run, and golem replaces the value with its reference everywhere it writes. For example, `t:"staging-password"` shows as `t:"${qa.password}"`.

Tag a value in one of these places:

- **A fixture.** A root `secret` list names the keys of `[vars]` that are secret. See [`load_fixture`](actions-reference.md#load_fixture--load-fixture-data).
- **A step with `save_to`.** `secret = true` tags the whole saved value. `secret = ["token", "user.refresh_token"]` tags only those paths, for example the token of a test user that an HTTP step creates. If a path is not in the saved value, the step passes with the warning WP463, because the value under the real path is not masked.
- **`create_inbox`.** The inbox password (`pass`) is always secret.

To keep a secret out of every file, get it from the environment or a secret manager with a step:

```toml
{ action = "bash", run = "op read op://qa/password", save_to = "pw", secret = true }
```

golem masks the value, not the variable. A string made from a secret, such as `"Bearer ${user.token}"`, shows as `Bearer ${user.token}`, and a response that sends the token back is masked too. golem masks a secret of any length. A short secret, such as a PIN, also masks the same text elsewhere in the output: the output is harder to read, but the secret does not show. For an object such as the result of `run`, name the paths: `secret = true` also masks fields such as `exit_code`.

The limits:

- golem cannot mask pixels. A screenshot or a screen recording shows a secret that the app displays. Password fields usually show dots.
- golem cannot mask a changed form of a value: base64, URL encoding, a hash, or a value split across lines.
- Masking hides a value. It does not stop a step from using the value: a step can still send it with `bash` or `post_http`.
- Use test accounts, not real users.
