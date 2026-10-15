## Install cache
<!-- summary: golem installs once per app and device, and when it installs again -->

Two layers of caching, both transparent in the default mode:

**In-memory (per suite)** — keyed on `(device_udid, bundle_id)`. Within a single `golem run`, the script runs at most once per combination.

- `Succeeded` → subsequent flows on the same device skip the script and go straight to launch
- `FailedScript` / `FailedNoScript` → subsequent flows using that combo are **skipped** with a clear reason (no repeated retries on broken setups)

**Persistent (cross-run)** — `.golem/install-cache.json`, keyed `(udid, bundle)` → `{ fingerprint, device_install_time, installed_version, installed_at }`. Subsequent `golem run` invocations skip both build AND install when **all three** integrity gates pass:

1. **Device-present** — device reports the bundle as installed (`xcrun simctl get_app_container` / `adb shell pm path`)
2. **Install-time matches** — device's bundle mtime / `lastUpdateTime` matches the cached `device_install_time`. Catches external reinstalls (Xcode "Run", manual `simctl install`)
3. **Fingerprint matches** — current source fingerprint equals the cached one. Tier 1: `git rev-parse HEAD` + sha1 of `git status --porcelain`. Tier 2 (non-git): content hash of the project tree honouring `.gitignore`

Any gate failing → cache miss, normal build+install runs, fresh entry written.

**On hit** the live stream prints `skipped (cache hit)` — terse. A hit always means **all three gates passed**; the source-fingerprint identity is implied (you almost always know what state your tree is in already, so listing it on every hit is noise).

**On miss** the stream prints a specific reason so you can see *why* a build was triggered. Examples:

- `cache miss on iPhone 17 — source fingerprint changed (git:abc → git:def)` — you committed / edited code (label shows clean→dirty or rev→rev movement)
- `cache miss on iPhone 17 — device install-time differs (... — external reinstall?)` — Xcode "Run", manual `simctl install`, or another tool replaced the binary
- `cache miss on iPhone 17 — bundle no longer installed on device` — sim was reset / app was uninstalled
- `cache miss on iPhone 17 — fingerprint unavailable (no git, no readable source tree)` — neither tier could compute a fingerprint (extremely rare)

The "no prior cache entry" case (first-time install on a fresh checkout) is silent — that's the normal path on a cold cache, not a cache invalidation worth flagging.

Where the label *does* render (in fingerprint-changed misses): clean trees show just the rev (`git:abc1234`); dirty trees include a 4-char porcelain-hash suffix (`git:abc1234+0a1b`) so two dirty trees with the same commit but different uncommitted edits render distinctly.
