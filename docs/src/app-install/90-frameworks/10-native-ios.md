### native-ios
<!-- summary: xcodebuild, then simctl or devicectl install -->

xcodebuild + `xcrun simctl install` (simulator) / `xcrun devicectl` or `ios-deploy` (physical). Reads these variables from `install_env` or the shell:
- `CONFIGURATION` and `XCODE_SCHEME`: override the values written into the script.
- `XCCONFIG`: an `.xcconfig` path, passed as `-xcconfig`.
- `DEVELOPMENT_TEAM`: your Apple team ID, for a physical device. The script passes it as a build setting with `-allowProvisioningUpdates`. If a physical-device build fails, the error names this variable. **The physical-device path is not yet validated (#59).**

The script fails if the `.app` is older than the build start.
