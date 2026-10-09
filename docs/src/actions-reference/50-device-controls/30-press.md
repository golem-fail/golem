### `press` — Press hardware button

```toml
{ action = "press", button = "home" }
{ action = "press", button = "back" }       # Android only
{ action = "press", button = "volume_up" }
```

**Supported buttons (platform-specific):**

| `button`      | Android (`input keyevent`) | iOS (`/press` → `XCUIDevice.press`) |
|---------------|----------------------------|-------------------------------------|
| `home`        | ✓ `HOME`                   | ✓ `.home`                           |
| `back`        | ✓ `BACK`                   | — (no hardware back button)         |
| `volume_up`   | ✓ `VOLUME_UP`              | —                                   |
| `volume_down` | ✓ `VOLUME_DOWN`            | —                                   |

An unsupported button errors at action time. On iOS only `home` exists;
`simctl ui … home` was dropped in Xcode 26, so golem drives it through the
companion's `/press` endpoint (`XCUIDevice.shared.press(.home)`), the
version-stable path.

App permissions are declared on a launch, not as a device control — see [App permissions](#app-permissions) under App Lifecycle.
