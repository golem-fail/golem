// Categorized failure codes: <severity><domain><3-digit cause>, e.g. EF404.
//
// Severity (E error / W warning) is derived from the step outcome at render
// time and never stored — the same cause renders EF404 under the default
// policy and WF404 under if_fail="warn". The domain letter maps a failure
// to the party responsible for triage; the cause number is stable across
// severities and mirrors HTTP semantics where natural (404 not found,
// 408 timeout, 409 busy, 503 unavailable).

use serde::{Deserialize, Serialize};

/// Responsibility domain — char 2 of the rendered code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    /// Toolchain / orchestrator host problems (SRE).
    Host,
    /// Device boot, companion, driver comms (device farm).
    Device,
    /// App build / install / launch (app dev).
    App,
    /// Runtime test logic — could be wrong test or wrong app
    /// (test writer / app dev).
    Flow,
    /// Test file, params, or suite config invalid (test writer).
    Parsing,
}

impl Domain {
    pub fn letter(self) -> char {
        match self {
            Self::Host => 'H',
            Self::Device => 'D',
            Self::App => 'A',
            Self::Flow => 'F',
            Self::Parsing => 'P',
        }
    }

    /// Whether this domain is an environment/infrastructure problem rather
    /// than a fault in the test or its spec. Host, Device, and App failures
    /// mean the test couldn't run properly (the environment broke); Flow and
    /// Parsing failures mean the test logic or its definition is wrong. Used
    /// by JUnit to map onto `<error>` (infrastructure) vs `<failure>` (test).
    pub fn is_infrastructure(self) -> bool {
        matches!(self, Self::Host | Self::Device | Self::App)
    }
}

/// Severity — char 1 of the rendered code. Derived from the outcome
/// (Failed → Error, Warning → Warning) at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn letter(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
        }
    }
}

/// A categorized failure cause. One variant per distinguishable cause
/// class — the single source of truth for the code registry. Multiple
/// origin sites share a variant when the triage action is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCode {
    // ── F: flow runtime ──
    /// F400: explicit `fail` action invoked.
    FlowExplicitFail,
    /// F404: element not found within timeout.
    FlowElementNotFound,
    /// F405: element exists but offscreen / scroll exhausted.
    FlowElementOffscreen,
    /// F408: step exceeded its effective timeout.
    FlowStepTimeout,
    /// F409: assert_not_visible — element still present.
    FlowUnexpectedlyPresent,
    /// F412: assertion mismatch (alert/text).
    FlowAssertionMismatch,
    /// F417: alert/dialog present but interaction failed (no/wrong buttons).
    FlowAlertInteraction,
    /// F424: external action failed (bash/run/http non-zero, await_email no match).
    FlowExternalFailed,
    /// F504: flow max_runtime exceeded.
    FlowMaxRuntime,
    /// F508: max_steps exceeded (loop guard).
    FlowMaxSteps,
    /// F000: fallback for errors that reached output without a tag —
    /// renders so coverage gaps stay visible.
    Uncoded,

    // ── P: parsing / test authoring ──
    /// P400: unknown action keyword.
    ParseUnknownAction,
    /// P404: missing reference — block, sub-flow, or fixture not found.
    ParseMissingReference,
    /// P422: required param missing or invalid (incl. gesture geometry).
    ParseMissingParam,
    /// P450: variable syntax/type error, unknown generator.
    ParseVariable,
    /// P460: flow file parse/mixin failure.
    ParseFlowFile,
    /// P461: suite device-constraint unsatisfiable.
    ParseDeviceConstraint,

    // ── A: app build / install / lifecycle ──
    /// A403: install script path traversal blocked.
    AppInstallPathBlocked,
    /// A404: install script/bundle not found.
    AppInstallScriptNotFound,
    /// A408: install timed out.
    AppInstallTimeout,
    /// A500: install failed (non-zero exit).
    AppInstallFailed,
    /// A502: app state query failed (post-install verify).
    AppStateQueryFailed,
    /// A503: app launch or stop failed.
    AppLifecycleFailed,

    // ── D: device / driver / companion ──
    /// D404: device not found / discovery failed.
    DeviceNotFound,
    /// D408: device boot timeout.
    DeviceBootTimeout,
    /// D409: device busy / --max-wait exceeded.
    DeviceBusy,
    /// D500: device/simulator creation failed.
    DeviceCreateFailed,
    /// D502: webview driver comms failed (CDP/WebKit inspector).
    DeviceWebviewComms,
    /// D503: companion wedged / hierarchy fetch timeout.
    DeviceCompanionWedged,
    /// D504: companion registration timeout.
    DeviceRegistrationTimeout,
    /// D520: misc driver op failed (adb forward, unsupported button).
    DeviceDriverOpFailed,

    // ── H: host / orchestrator ──
    /// H404: toolchain/artifact missing (avdmanager, iOS runtimes, companion binary).
    HostToolchainMissing,
    /// H429: port allocation exhausted.
    HostPortsExhausted,
    /// H502: orchestrator socket/IPC failure.
    HostOrchestratorIpc,
}

impl FailureCode {
    pub fn domain(self) -> Domain {
        use FailureCode::*;
        match self {
            FlowExplicitFail | FlowElementNotFound | FlowElementOffscreen
            | FlowStepTimeout | FlowUnexpectedlyPresent | FlowAssertionMismatch
            | FlowAlertInteraction | FlowExternalFailed | FlowMaxRuntime
            | FlowMaxSteps | Uncoded => Domain::Flow,
            ParseUnknownAction | ParseMissingReference | ParseMissingParam
            | ParseVariable | ParseFlowFile | ParseDeviceConstraint => Domain::Parsing,
            AppInstallPathBlocked | AppInstallScriptNotFound | AppInstallTimeout
            | AppInstallFailed | AppStateQueryFailed | AppLifecycleFailed => Domain::App,
            DeviceNotFound | DeviceBootTimeout | DeviceBusy | DeviceCreateFailed
            | DeviceWebviewComms | DeviceCompanionWedged
            | DeviceRegistrationTimeout | DeviceDriverOpFailed => Domain::Device,
            HostToolchainMissing | HostPortsExhausted | HostOrchestratorIpc => Domain::Host,
        }
    }

    pub fn number(self) -> u16 {
        use FailureCode::*;
        match self {
            FlowExplicitFail => 400,
            FlowElementNotFound => 404,
            FlowElementOffscreen => 405,
            FlowStepTimeout => 408,
            FlowUnexpectedlyPresent => 409,
            FlowAssertionMismatch => 412,
            FlowAlertInteraction => 417,
            FlowExternalFailed => 424,
            FlowMaxRuntime => 504,
            FlowMaxSteps => 508,
            Uncoded => 0,
            ParseUnknownAction => 400,
            ParseMissingReference => 404,
            ParseMissingParam => 422,
            ParseVariable => 450,
            ParseFlowFile => 460,
            ParseDeviceConstraint => 461,
            AppInstallPathBlocked => 403,
            AppInstallScriptNotFound => 404,
            AppInstallTimeout => 408,
            AppInstallFailed => 500,
            AppStateQueryFailed => 502,
            AppLifecycleFailed => 503,
            DeviceNotFound => 404,
            DeviceBootTimeout => 408,
            DeviceBusy => 409,
            DeviceCreateFailed => 500,
            DeviceWebviewComms => 502,
            DeviceCompanionWedged => 503,
            DeviceRegistrationTimeout => 504,
            DeviceDriverOpFailed => 520,
            HostToolchainMissing => 404,
            HostPortsExhausted => 429,
            HostOrchestratorIpc => 502,
        }
    }

    /// Severity-less fragment, e.g. `F404`. Used as the `CodedError`
    /// Display so untagged `{e:#}` call sites degrade to `F404: msg`.
    pub fn fragment(self) -> String {
        format!("{}{:03}", self.domain().letter(), self.number())
    }

    /// Full rendered code with severity prefix, e.g. `EF404` / `WF404`.
    pub fn render(self, sev: Severity) -> String {
        format!("{}{}", sev.letter(), self.fragment())
    }
}

// ── anyhow integration ──

/// Tag carried in an error chain to record a failure's cause code. The code
/// is surfaced *structurally* (by downcasting `chain()` items) — never as
/// message text. `Display` is the original error message verbatim, so
/// existing `{e}` / `{e:#}` call sites keep printing the human-readable
/// reason rather than a bare code fragment.
#[derive(Debug)]
pub struct CodedError {
    pub code: FailureCode,
    /// The full rendered message of the error being tagged (`{e:#}` of the
    /// inner error), captured at tag time. Flattening to a string keeps the
    /// tag a chain leaf, so `{e:#}` never double-prints the cause.
    message: String,
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

/// Tag a `Result`'s error with a failure code: `Err(anyhow!("..")).code(F404)`.
pub trait CodeExt<T> {
    fn code(self, c: FailureCode) -> anyhow::Result<T>;
}

impl<T, E: Into<anyhow::Error>> CodeExt<T> for Result<T, E> {
    fn code(self, c: FailureCode) -> anyhow::Result<T> {
        self.map_err(|e| coded(c, e.into()))
    }
}

/// Tag an error value directly (for non-`Result` construction sites).
pub fn coded(c: FailureCode, e: anyhow::Error) -> anyhow::Error {
    anyhow::Error::new(CodedError { code: c, message: format!("{e:#}") })
}

/// Walk the error chain for the outermost `CodedError` tag.
pub fn extract_code(e: &anyhow::Error) -> Option<FailureCode> {
    e.chain().find_map(|c| c.downcast_ref::<CodedError>().map(|m| m.code))
}

/// The human-readable message for an error, identical to `{e:#}`. `CodedError`
/// renders its message transparently, so the code fragment never leaks into
/// the text — extract it separately via [`extract_code`].
pub fn clean_msg(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn render_severity_and_domain_letters() {
        assert_eq!(FailureCode::FlowStepTimeout.render(Severity::Error), "EF408");
        assert_eq!(FailureCode::FlowStepTimeout.render(Severity::Warning), "WF408");
        assert_eq!(FailureCode::ParseMissingParam.render(Severity::Error), "EP422");
        assert_eq!(FailureCode::AppInstallFailed.render(Severity::Error), "EA500");
        assert_eq!(FailureCode::DeviceCompanionWedged.render(Severity::Error), "ED503");
        assert_eq!(FailureCode::HostToolchainMissing.render(Severity::Error), "EH404");
    }

    #[test]
    fn uncoded_renders_zero_padded() {
        assert_eq!(FailureCode::Uncoded.render(Severity::Error), "EF000");
        assert_eq!(FailureCode::Uncoded.render(Severity::Warning), "WF000");
    }

    #[test]
    fn number_stable_across_severities() {
        let c = FailureCode::FlowElementNotFound;
        assert_eq!(c.number(), 404);
        assert_eq!(&c.render(Severity::Error)[1..], &c.render(Severity::Warning)[1..]);
    }

    #[test]
    fn code_ext_roundtrip() {
        let r: anyhow::Result<()> =
            Err(anyhow!("no element found")).code(FailureCode::FlowElementNotFound);
        let e = match r {
            Err(e) => e,
            Ok(()) => panic!("expected Err"),
        };
        assert_eq!(extract_code(&e), Some(FailureCode::FlowElementNotFound));
    }

    #[test]
    fn extract_survives_outer_context() {
        let r: anyhow::Result<()> =
            Err(anyhow!("inner")).code(FailureCode::FlowStepTimeout);
        let e = match r {
            Err(e) => e.context("while running step"),
            Ok(()) => panic!("expected Err"),
        };
        assert_eq!(extract_code(&e), Some(FailureCode::FlowStepTimeout));
        assert_eq!(clean_msg(&e), "while running step: inner");
    }

    #[test]
    fn clean_msg_excludes_marker() {
        let r: anyhow::Result<()> =
            Err(anyhow!("Step timed out after 10000ms")).code(FailureCode::FlowStepTimeout);
        let e = match r {
            Err(e) => e,
            Ok(()) => panic!("expected Err"),
        };
        let msg = clean_msg(&e);
        assert_eq!(msg, "Step timed out after 10000ms");
        assert!(!msg.contains("F408"));
    }

    #[test]
    fn untagged_error_yields_none() {
        let e = anyhow!("plain error");
        assert_eq!(extract_code(&e), None);
        assert_eq!(clean_msg(&e), "plain error");
    }
}
