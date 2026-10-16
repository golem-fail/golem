## Cache flags: `--rebuild` and `--no-build`
<!-- summary: --rebuild forces an install; --no-build skips it -->

Two flags control cache behaviour. Default (no flag) = strict mode: read cache, skip on full gate match, write after build.

**`--rebuild`** — force a fresh build for this run. Bypasses cache reads so every `(device, bundle)` is rebuilt + reinstalled. The cache is **still written** after a successful build, so the next run benefits. Use when:

- A flaky build produced a bad binary and you want to start clean
- You suspect the cache is wrong but don't want to manually delete it
- You're verifying a CI build matches what local develops

```bash
golem run flows/ --rebuild
```

**`--no-build`** — skip build+install entirely. The cache is **not consulted and not written**. For each `(device, bundle)`:

- Device has the bundle → flow runs against the existing binary
- Device missing the bundle → flow fails immediately with `--no-build: <bundle> not installed on <device>; drop --no-build or install manually`

Use when:

- Iterating on flow files only — the binary hasn't changed and you want to skip even the ~150ms cache check
- You built manually via Xcode / Android Studio and want golem to test against that

```bash
golem run flows/ --no-build
```

**Both passed** — `--no-build` wins; golem emits a warning. The two intents are mutually exclusive (force rebuild vs trust device), so passing both is almost always a mistake.
