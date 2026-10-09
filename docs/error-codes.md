<!-- Generated from docs/src/error-codes/ — edit the parts there, then run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs`. -->
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

<!-- Generated from FailureCode in golem-events/src/code.rs (meaning, fix). -->

| Code | Meaning | Fix |
|------|---------|-----|
| `F400` | A `fail` step ran. | Check the branch or condition that reached the `fail` step. |
| `F404` | No visible element matched the selector before the timeout. | Fix the selector, add `auto_scroll = true`, or wait for the screen first. |
| `F405` | The element exists, but scrolling did not bring it into view. | Set `within` to the scroll container that holds the element. |
| `F408` | The step ran past its timeout. | Raise the step's `timeout`; if many steps time out, check the host and device. |
| `F409` | `assert_not_visible`: the element is still present. | Wait for the screen change first, or report the app bug. |
| `F412` | The alert or text did not match the expected value. | Fix the expected value, or report the app bug. |
| `F417` | An alert is shown, but golem could not press its button. | Name a button that the alert has. |
| `F424` | A `bash`, `run` or HTTP step failed, or `await_email` found no match. | Read the output or response; fix the script, endpoint or email filter. |
| `F504` | The flow ran past `max_runtime`. | Raise `max_runtime`, or find the slow or looping blocks. |
| `F508` | The flow ran more than `max_steps` steps, usually in a loop. | Fix the `next` or branch loop, or raise `max_steps`. |
| `X000` | golem did not classify this error. | Read the message, and report it to the golem maintainers. |
| `P400` | The `action` is not a known action. | Fix the spelling; see the actions reference. |
| `P404` | A block, subflow or fixture that the flow names does not exist. | Fix the name or path in `next`, `goto`, `run_flow` or `load_fixture`. |
| `P422` | A required field is missing or not valid. | Add or fix the field that the message names. |
| `P450` | A `${…}` reference has bad syntax or type, or names an unknown generator. | Fix the variable reference or the generator name. |
| `P460` | The flow or mixin file is not valid TOML, or does not fit the schema. | Fix the file at the line that the message names. |
| `P461` | No device can satisfy the flow's device constraints. | Relax the device constraints, or add a device that matches. |
| `P462` | `add_media` got a file that is not a supported image or video. | Use a supported image or video file. |
| `A403` | The install script path is outside the project. | Keep `install_script` inside the project directory. |
| `A404` | The install script or the app bundle does not exist. | Fix the `install_script` or bundle path, or build the app first. |
| `A408` | The app install ran past its timeout. | Make the install faster, or raise `install_timeout_ms`. |
| `A500` | The install script exited with an error. | Read the script output, and fix the build or the install. |
| `A501` | The app shows a React Native error overlay instead of its UI. | Fix the JavaScript error that the overlay names. |
| `A502` | golem could not read the app's state after the install. | Check that the app installed; install it again. |
| `A503` | The app did not launch or stop. | Check the bundle id and the app's crash log (`app_logs`). |
| `D404` | No device matches. | Boot or connect a device; `golem devices` lists them. |
| `D408` | The device did not finish booting in time. | Boot the simulator or emulator by hand once, and check its image. |
| `D409` | Every matching device stayed busy past `--max-device-wait`. | Free a device, add a device, or raise `--max-device-wait`. |
| `D500` | golem could not create the simulator or emulator. | Install the runtime or system image that the device needs. |
| `D502` | golem could not talk to the webview inspector. | Make the webview debuggable, then try again. |
| `D503` | The companion runs but is stuck on a call. | Try again; if it repeats, restart the device or the app. |
| `D504` | The companion did not register with golem in time. | Check that the companion installed and launched; install it again. |
| `D505` | The companion refused the connection; it is not running. | Try again; if it repeats, reduce the host load or restart the device. |
| `D506` | The companion stopped after each restart, so golem gave up. | Restart the device, and check the host's free memory and CPU. |
| `D507` | The companion connection closed during a request. | Check whether the step had its effect, then try again. |
| `D520` | A device driver operation failed. | Read the message; for `press`, use a button that the device has. |
| `H404` | A tool or file that golem needs is missing. | Install what the message names; `golem doctor` checks the host. |
| `H429` | golem has no free ports. | Run fewer devices at once, or free ports. |
| `H502` | The connection to the golem daemon failed. | Restart the daemon, and check `GOLEM_SOCKET`. |
| `H424` | `browse_*` steps found no Chrome or Chromium. | Install Chrome, or set `$CHROME` to the browser binary. |
| `H501` | This golem build has no browser support. | Use a default build (without `--no-default-features`). |
| `H503` | `--dev`: no Expo or Metro dev server answers. | Start the dev server (`npx expo start`), or set `--dev-port`. |
| `H505` | The browser lacks a feature that the flow needs (WebMCP). | Upgrade the browser; `golem doctor` checks it. |
