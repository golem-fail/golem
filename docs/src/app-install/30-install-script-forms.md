## `install_script` forms
<!-- summary: One script for both platforms, or one for each platform -->

Either a single path (cross-platform):

```toml
install_script = "scripts/install.sh"
```

Or a platform-keyed table (native separate iOS + Android builds):

```toml
install_script = { ios = "scripts/install-ios.sh", android = "scripts/install-android.sh" }
```

Golem picks the right entry for the target platform at run time.
