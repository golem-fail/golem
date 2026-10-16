## Install cache
<!-- summary: golem installs once per app and device, and when it installs again -->

**In one run,** golem runs an app's install script at most once per device. After a failed install, the other flows for that app and device skip, with the reason.

**Across runs,** `.golem/install-cache.json` remembers each install. A later `golem run` skips the build and the install when all three checks pass:

1. The app is on the device (`simctl get_app_container`, `adb shell pm path`).
2. The app's install time on the device is the cached one. A reinstall by another tool (Xcode Run, `simctl install`) fails this check.
3. The source fingerprint is the cached one: `git rev-parse HEAD` plus a hash of `git status --porcelain`, or without git, a hash of the files that `.gitignore` does not exclude.

A hit prints `skipped (cache hit)`. A miss builds and installs, and prints why:

- `source fingerprint changed (git:abc → git:def)`: a commit or an edit. A dirty tree shows a suffix, `git:abc1234+0a1b`.
- `device install-time differs`: another tool reinstalled the app.
- `bundle no longer installed on device`: the simulator was reset, or the app was removed.
- `fingerprint unavailable`: golem could not compute one.

A first install, with no cache entry, prints no reason.
