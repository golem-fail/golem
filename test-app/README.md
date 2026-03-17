# GOLEM Test App

A reference application used for end-to-end testing of the GOLEM mobile UI testing framework. This app provides a comprehensive set of UI elements with stable accessibility IDs that can be targeted by GOLEM test scripts.

## Building

The test app is built with [Tauri](https://tauri.app/) targeting iOS and Android. To build:

```bash
# Install Tauri CLI
cargo install tauri-cli

# iOS
cd test-app/src-tauri
cargo tauri ios build

# Android
cargo tauri android build
```

The app's bundle identifiers:
- iOS: `com.golem.testapp`
- Android: `com.golem.testapp`

## URL Scheme

The app registers the deep link scheme `golem-test://`. Any path opened via this scheme is displayed in the `deeplink-display` label.

## UI Elements Reference

All elements listed below have stable accessibility identifiers that persist across app launches and OS versions. These IDs are used by GOLEM test scripts to locate and interact with elements.

### Text Fields

| Accessibility ID   | Type      | Placeholder      | Notes                  |
|---------------------|-----------|------------------|------------------------|
| `email-input`       | TextField | "Enter email"    | Standard email field   |
| `password-input`    | TextField | "Enter password" | Secure text entry      |
| `search-input`      | TextField | "Search..."      | Search bar             |
| `multiline-input`   | TextArea  | (none)           | Multiline text area    |

### Buttons

| Accessibility ID     | Type   | Text       | Notes                           |
|-----------------------|--------|------------|---------------------------------|
| `submit-button`       | Button | "Submit"   | Enabled                         |
| `disabled-button`     | Button | "Disabled" | Always disabled                 |
| `duplicate-button-1`  | Button | "Action"   | Same text as duplicate-button-2 |
| `duplicate-button-2`  | Button | "Action"   | Same text as duplicate-button-1 |

### Toggles and Switches

| Accessibility ID      | Type     | Text                  | Notes                |
|------------------------|----------|-----------------------|----------------------|
| `dark-mode-toggle`     | Switch   | "Dark Mode"           | Toggle switch        |
| `notifications-toggle` | Switch   | "Notifications"       | Toggle switch        |
| `agree-checkbox`       | CheckBox | "I agree to terms"    | Checkbox             |

### Counter

| Accessibility ID   | Type   | Text | Notes                       |
|---------------------|--------|------|-----------------------------|
| `counter-display`   | Label  | "0"  | Shows current count         |
| `increment-button`  | Button | "+"  | Increments counter          |
| `decrement-button`  | Button | "-"  | Decrements counter          |

### Scrollable List

| Accessibility ID       | Type       | Text         | Notes                       |
|-------------------------|------------|--------------|-----------------------------|
| `scroll-list`           | ScrollView | (none)       | Vertical scrollable list    |
| `list-item-0` ... `49` | Label      | "Item 0"...  | 50 items in the list        |

### Horizontal Carousel

| Accessibility ID           | Type       | Text         | Notes                      |
|-----------------------------|------------|--------------|----------------------------|
| `carousel`                  | ScrollView | (none)       | Horizontal scrollable      |
| `carousel-item-0` ... `9`  | View       | "Card 0"...  | 10 carousel cards          |

### Nested Layout (for relational selectors)

| Accessibility ID  | Type | Contains        | Notes                |
|--------------------|------|-----------------|----------------------|
| `header-section`   | View | "App Title"     | Top section          |
| `content-section`  | View | nested items    | Middle section       |
| `footer-section`   | View | "Footer Text"   | Bottom section       |
| `left-panel`       | View | sidebar items   | Left sidebar         |
| `right-panel`      | View | main content    | Right content area   |

### Alert Triggers

| Accessibility ID      | Type   | Text             | Notes                        |
|------------------------|--------|------------------|------------------------------|
| `alert-button`         | Button | "Show Alert"     | Triggers system alert        |
| `confirm-button`       | Button | "Confirm"        | Triggers confirmation dialog |
| `action-sheet-button`  | Button | "Action Sheet"   | Triggers action sheet        |

### Device State Labels

| Accessibility ID           | Type  | Text              | Notes                            |
|-----------------------------|-------|-------------------|----------------------------------|
| `orientation-label`         | Label | "Portrait"        | Updates on rotation              |
| `theme-label`               | Label | "Light"           | Updates with dark mode           |
| `location-label`            | Label | "0.0, 0.0"        | Updates with mock location       |
| `deeplink-display`          | Label | ""                 | Shows deep link path             |
| `notification-display`      | Label | ""                 | Shows last notification payload  |
| `media-count-label`         | Label | "0"                | Shows count of received media    |

### Permission Buttons

| Accessibility ID            | Type   | Text                    | Notes                    |
|------------------------------|--------|-------------------------|--------------------------|
| `camera-permission-button`   | Button | "Request Camera"        | Requests camera access   |
| `location-permission-button` | Button | "Request Location"      | Requests location access |

## Hierarchy Fixture

The file `hierarchy-fixture.json` contains a complete UI hierarchy snapshot in the format returned by the GOLEM companion server's `/hierarchy` endpoint. This fixture is loaded by `MockPlatformDriver` during E2E tests so that tests can run without a physical device.

The file `ui-spec.json` contains a machine-readable specification of all UI elements, their types, expected text, and accessibility IDs. This can be used for test generation and validation.
