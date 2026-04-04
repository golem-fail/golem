# GOLEM Test App B

A minimal companion app used for multi-app testing with GOLEM. This app is the second target in `[[flow.apps]]` configurations, allowing tests to verify app-switching, deep link routing, and shared data passing between apps.

## Bundle ID

- `fail.golem.test-b`

## URL Scheme

`golem-test-b://` — any path opened via this scheme is displayed in the `deeplink-display-b` label.

## UI Elements

| Accessibility ID      | Type   | Text / Default   | Notes                            |
|------------------------|--------|------------------|----------------------------------|
| `app-b-title`          | Label  | "GOLEM Test B"   | Title text                       |
| `shared-data-display`  | Label  | ""               | Shows shared data received from app A |
| `refresh-button`       | Button | "Refresh"        | Triggers a data refresh          |
| `status-label`         | Label  | "Ready"          | Shows "Ready" or "Updated"       |
| `deeplink-display-b`   | Label  | ""               | Shows received deep link path    |

## Hierarchy Fixture

The file `hierarchy-fixture.json` contains a UI hierarchy snapshot matching the Element struct format used by `MockPlatformDriver` during E2E tests.
