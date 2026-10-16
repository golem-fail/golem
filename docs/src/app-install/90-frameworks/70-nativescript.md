### nativescript
<!-- summary: ns build, then install the artifact -->

NativeScript 8+. One cross-platform script: it installs JS dependencies when they changed, runs `ns build <platform>`, then installs the artifact that build wrote with `simctl` / `devicectl` or `adb`. It does not use `ns build --copy-to`, which covers only the device `.ipa` and has been broken for simulator builds. The iOS `.app` is looked up under both `platforms/ios/build/` and `platforms/ios/Build/`, because CLI versions differ.

**Host needs:** the NativeScript CLI, as a `nativescript` devDependency (run with `npx ns`) or a global `ns`, and CocoaPods + Ruby for iOS.

The script reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: `debug` (default) or `release` (adds `--release`).
- `NS_BUILD_ARGS`: extra `ns build` flags, for example `--env.*` bundler flags, or the `--key-store-*` flags that an Android release build needs for signing.
- `DEVELOPMENT_TEAM` (`--team-id`) or `PROVISION` (`--provision`): signing for a physical iOS device, which builds with `--for-device`.

**Selectors:** NativeScript renders real native views, so text selectors work on both platforms. `automationText` (and `testID`) set the accessibility *identifier*. On iOS golem matches it with `accessibility_label`. On Android it is only a view tag, not a `resource-id`, so the accessibility tree does not carry it and golem cannot see it. To give an Android view a label golem can match, set `accessibilityLabel` (Android's `contentDescription`). On iOS that label replaces the view's visible text.
