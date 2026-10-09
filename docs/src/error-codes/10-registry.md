## Registry

| Code | Meaning |
|------|---------|
| `F400` | Explicit `fail` action invoked |
| `F404` | Element not found within timeout |
| `F405` | Element exists but off-screen / scroll exhausted |
| `F408` | Step exceeded its timeout |
| `F409` | `assert_not_visible`: element still present |
| `F412` | Assertion mismatch (alert / text) |
| `F417` | Alert/dialog present but interaction failed |
| `F424` | External action failed (bash / run / http / await_email) |
| `F504` | Flow `max_runtime` exceeded |
| `F508` | `max_steps` exceeded |
| `P400` | Unknown action keyword |
| `P404` | Missing reference — block, sub-flow, or fixture |
| `P422` | Required param missing or invalid (incl. gesture geometry, empty selector) |
| `P450` | Variable syntax/type error, unknown generator |
| `P460` | Flow file parse / mixin failure |
| `P461` | Suite device-constraint unsatisfiable |
| `A403` | Install script path traversal blocked |
| `A404` | Install script / bundle not found |
| `A408` | Install timed out |
| `A500` | Install failed (non-zero exit) |
| `A501` | The app is showing a React Native error overlay (native redbox or JS LogBox) instead of its UI. Fix the error the message names; reported instead of letting it read as a missing selector |
| `A502` | App state query failed (post-install verify) |
| `A503` | App launch / stop failed |
| `D404` | Device not found / discovery failed |
| `D408` | Device boot timeout |
| `D409` | Device busy / `--max-device-wait` exceeded |
| `D500` | Device / simulator creation failed |
| `D502` | Webview driver comms failed (CDP / WebKit) |
| `D503` | Companion wedged — alive but a main-thread call is stuck (incl. a `504` from the companion's own watchdog, or a client-side request timeout) |
| `D504` | Companion registration timeout |
| `D505` | Companion unreachable — connection refused mid-request (process gone / not yet accepting); death or cold-start drop |
| `D520` | Driver op failed (adb forward, unsupported button) |
| `H404` | Toolchain / artifact missing (avdmanager, iOS runtime, companion binary) |
| `H424` | No Chrome / Chromium found for `browse_*` steps — install one, or point `$CHROME` at the binary |
| `H429` | Port allocation exhausted |
| `H501` | Flow uses `browse_*` but this golem was built `--no-default-features` (no `browser`) |
| `H502` | Orchestrator socket / IPC failure |
| `H503` | `--dev`: no dev server (Expo/Metro) answering — start it (`npx expo start`), or point `--dev-port` at the right port |
| `H505` | The browser lacks a feature the flow needs (WebMCP) — upgrade it, or check `golem doctor` |
| `X000` | Uncoded failure — unclassified, reached output without a domain tag |
