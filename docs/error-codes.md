# Error Codes

*Every failure and warning carries a short code so you can grep, triage, and route by who owns the fix.*

← [Back to README](../README.md)

A code is five characters: `<severity><domain><number>`, e.g. `EF408`.

- **Severity** (1st char): `E` for a failure, `W` for a warning. The same underlying cause is `E` by default and `W` when the step sets `if_fail = "warn"`.
- **Domain** (2nd char): who is most likely responsible.

  | Letter | Domain | Owner |
  |--------|--------|-------|
  | `H` | Host — toolchain, orchestrator, ports | SRE / CI |
  | `D` | Device — boot, companion, driver comms | Device farm |
  | `A` | App — build, install, launch | App developer |
  | `F` | Flow — runtime test logic | Test writer / app developer¹ |
  | `P` | Parsing — test file, params, suite config | Test writer |
  | `X` | Unknown — unclassified, the engine didn't tag it | golem maintainers² |

  ¹ An `F` failure can mean the test is wrong *or* the app is wrong — golem can't always tell which.

  ² An `X` code is a coverage gap: an error reached output without a domain tag. It's deliberately *not* folded into `F`, so untagged failures stay visible rather than masquerading as test-logic faults.

- **Number** (last 3 chars): the specific cause, stable across `E`/`W`.

An uncoded failure renders `EX000` (or `WX000`) rather than no code, so coverage gaps stay visible.

Codes appear in every output format:
- **human**: prefixes the failure-detail line — `╰ EF408 Step timed out after 10000ms` — and the flow `FAIL` summary line, between the flow name and seed.
- **json**: a `"code"` string field on each failed/warned step and on the flow.
- **toon**: a code token after `d:<ms>` — ` !tap:Login d:10003 EF408 Step timed out...`.
- **junit**: the `type` attribute of `<failure type="EF408" …>`; warnings prefix `[WF…]` in `system-out`.

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
| `P422` | Required param missing or invalid (incl. gesture geometry) |
| `P450` | Variable syntax/type error, unknown generator |
| `P460` | Flow file parse / mixin failure |
| `P461` | Suite device-constraint unsatisfiable |
| `A403` | Install script path traversal blocked |
| `A404` | Install script / bundle not found |
| `A408` | Install timed out |
| `A500` | Install failed (non-zero exit) |
| `A502` | App state query failed (post-install verify) |
| `A503` | App launch / stop failed |
| `D404` | Device not found / discovery failed |
| `D408` | Device boot timeout |
| `D409` | Device busy / `--max-wait` exceeded |
| `D500` | Device / simulator creation failed |
| `D502` | Webview driver comms failed (CDP / WebKit) |
| `D503` | Companion wedged / hierarchy fetch timeout |
| `D504` | Companion registration timeout |
| `D520` | Driver op failed (adb forward, unsupported button) |
| `H404` | Toolchain / artifact missing (avdmanager, iOS runtime, companion binary) |
| `H429` | Port allocation exhausted |
| `H502` | Orchestrator socket / IPC failure |
| `X000` | Uncoded failure — unclassified, reached output without a domain tag |
