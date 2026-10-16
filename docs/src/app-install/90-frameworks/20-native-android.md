### native-android
<!-- summary: Gradle assemble, then adb install -->

`./gradlew :<module>:assemble<Flavor><BuildType>`, then `adb -s <serial> install` of the APK that build wrote. Reads these variables from `install_env` or the shell:
- `BUILD_TYPE`: the Gradle build type, `debug` by default. A release build needs a `signingConfig`, because Android installs only signed APKs.
- `FLAVOR`: the product flavor. With more than one flavor dimension, use the full flavor name, for example `freeProd`.

The script fails if the APK is older than the build start. If `aapt2` is found under `ANDROID_HOME/build-tools`, the script also fails when the APK's `applicationId` is not the app's `bundle`.
