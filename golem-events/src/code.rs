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
    /// Unclassified — an error reached output without being tagged with a
    /// cause. Ownership is genuinely unknown (the engine didn't classify it),
    /// so it gets its own domain rather than being guessed into Flow. A live
    /// `X` code is a coverage gap: something threw without a `.code(...)`.
    Unknown,
}

impl Domain {
    pub fn letter(self) -> char {
        match self {
            Self::Host => 'H',
            Self::Device => 'D',
            Self::App => 'A',
            Self::Flow => 'F',
            Self::Parsing => 'P',
            Self::Unknown => 'X',
        }
    }

    /// Whether this domain is an environment/infrastructure problem rather
    /// than a fault in the test or its spec. Host, Device, and App failures
    /// mean the test couldn't run properly (the environment broke); Flow and
    /// Parsing failures mean the test logic or its definition is wrong. Used
    /// by JUnit to map onto `<error>` (infrastructure) vs `<failure>` (test).
    /// `Unknown` is reported as a test-side `<failure>` (we can't claim the
    /// environment broke without evidence).
    pub fn is_infrastructure(self) -> bool {
        matches!(self, Self::Host | Self::Device | Self::App)
    }
}

/// Severity — char 1 of the rendered code. Derived from the outcome
/// (Failed → Error, Warning → Warning) at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    /// X000: fallback for errors that reached output without a tag —
    /// renders in the `Unknown` domain so coverage gaps stay visible
    /// instead of being absorbed into the Flow bucket.
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
            FlowExplicitFail
            | FlowElementNotFound
            | FlowElementOffscreen
            | FlowStepTimeout
            | FlowUnexpectedlyPresent
            | FlowAssertionMismatch
            | FlowAlertInteraction
            | FlowExternalFailed
            | FlowMaxRuntime
            | FlowMaxSteps => Domain::Flow,
            Uncoded => Domain::Unknown,
            ParseUnknownAction
            | ParseMissingReference
            | ParseMissingParam
            | ParseVariable
            | ParseFlowFile
            | ParseDeviceConstraint => Domain::Parsing,
            AppInstallPathBlocked
            | AppInstallScriptNotFound
            | AppInstallTimeout
            | AppInstallFailed
            | AppStateQueryFailed
            | AppLifecycleFailed => Domain::App,
            DeviceNotFound
            | DeviceBootTimeout
            | DeviceBusy
            | DeviceCreateFailed
            | DeviceWebviewComms
            | DeviceCompanionWedged
            | DeviceRegistrationTimeout
            | DeviceDriverOpFailed => Domain::Device,
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
    anyhow::Error::new(CodedError {
        code: c,
        message: format!("{e:#}"),
    })
}

/// Walk the error chain for the outermost `CodedError` tag.
pub fn extract_code(e: &anyhow::Error) -> Option<FailureCode> {
    e.chain()
        .find_map(|c| c.downcast_ref::<CodedError>().map(|m| m.code))
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
        assert_eq!(
            FailureCode::FlowStepTimeout.render(Severity::Error),
            "EF408"
        );
        assert_eq!(
            FailureCode::FlowStepTimeout.render(Severity::Warning),
            "WF408"
        );
        assert_eq!(
            FailureCode::ParseMissingParam.render(Severity::Error),
            "EP422"
        );
        assert_eq!(
            FailureCode::AppInstallFailed.render(Severity::Error),
            "EA500"
        );
        assert_eq!(
            FailureCode::DeviceCompanionWedged.render(Severity::Error),
            "ED503"
        );
        assert_eq!(
            FailureCode::HostToolchainMissing.render(Severity::Error),
            "EH404"
        );
    }

    #[test]
    fn uncoded_renders_zero_padded() {
        assert_eq!(FailureCode::Uncoded.render(Severity::Error), "EX000");
        assert_eq!(FailureCode::Uncoded.render(Severity::Warning), "WX000");
    }

    #[test]
    fn number_stable_across_severities() {
        let c = FailureCode::FlowElementNotFound;
        assert_eq!(c.number(), 404);
        assert_eq!(
            &c.render(Severity::Error)[1..],
            &c.render(Severity::Warning)[1..]
        );
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
        let r: anyhow::Result<()> = Err(anyhow!("inner")).code(FailureCode::FlowStepTimeout);
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

    // 1. Each domain maps to its documented char-2 letter.
    #[test]
    fn domain_letters_are_distinct_and_correct() {
        assert_eq!(Domain::Host.letter(), 'H', "Host SHALL render as H");
        assert_eq!(Domain::Device.letter(), 'D', "Device SHALL render as D");
        assert_eq!(Domain::App.letter(), 'A', "App SHALL render as A");
        assert_eq!(Domain::Flow.letter(), 'F', "Flow SHALL render as F");
        assert_eq!(Domain::Parsing.letter(), 'P', "Parsing SHALL render as P");
    }

    // 2. Host/Device/App are infrastructure; Flow/Parsing are test faults.
    #[test]
    fn is_infrastructure_partitions_domains() {
        assert!(
            Domain::Host.is_infrastructure(),
            "Host SHALL be infrastructure"
        );
        assert!(
            Domain::Device.is_infrastructure(),
            "Device SHALL be infrastructure"
        );
        assert!(
            Domain::App.is_infrastructure(),
            "App SHALL be infrastructure"
        );
        assert!(
            !Domain::Flow.is_infrastructure(),
            "Flow SHALL NOT be infrastructure"
        );
        assert!(
            !Domain::Parsing.is_infrastructure(),
            "Parsing SHALL NOT be infrastructure"
        );
    }

    // 3. Severity letters cover both variants explicitly.
    #[test]
    fn severity_letters() {
        assert_eq!(Severity::Error.letter(), 'E', "Error SHALL render as E");
        assert_eq!(Severity::Warning.letter(), 'W', "Warning SHALL render as W");
    }

    // 4. A representative of each domain group maps back to the right Domain.
    #[test]
    fn domain_classification_per_group() {
        assert_eq!(FailureCode::FlowExplicitFail.domain(), Domain::Flow);
        assert_eq!(FailureCode::Uncoded.domain(), Domain::Unknown);
        assert_eq!(FailureCode::ParseUnknownAction.domain(), Domain::Parsing);
        assert_eq!(FailureCode::AppInstallPathBlocked.domain(), Domain::App);
        assert_eq!(FailureCode::DeviceNotFound.domain(), Domain::Device);
        assert_eq!(FailureCode::HostOrchestratorIpc.domain(), Domain::Host);
    }

    // 5. fragment is the severity-less domain-letter + zero-padded number.
    #[test]
    fn fragment_omits_severity_and_zero_pads() {
        assert_eq!(FailureCode::FlowElementNotFound.fragment(), "F404");
        assert_eq!(FailureCode::Uncoded.fragment(), "X000");
        assert_eq!(FailureCode::ParseMissingParam.fragment(), "P422");
        assert_eq!(FailureCode::DeviceDriverOpFailed.fragment(), "D520");
    }

    // 6. render composes severity letter + domain letter + zero-padded number,
    //    pinned to literals so a wrong order or missing separator is caught.
    #[test]
    fn render_is_severity_letter_plus_fragment() {
        let c = FailureCode::AppLifecycleFailed;
        assert_eq!(
            c.render(Severity::Error),
            "EA503",
            "render SHALL be severity letter + domain letter + number"
        );
        assert_eq!(
            c.render(Severity::Warning),
            "WA503",
            "warning severity SHALL only swap the leading letter"
        );
    }

    // 7. Spot-check the remaining numeric codes not exercised by render tests.
    #[test]
    fn numbers_match_registry() {
        assert_eq!(FailureCode::FlowExplicitFail.number(), 400);
        assert_eq!(FailureCode::FlowElementOffscreen.number(), 405);
        assert_eq!(FailureCode::FlowUnexpectedlyPresent.number(), 409);
        assert_eq!(FailureCode::FlowAssertionMismatch.number(), 412);
        assert_eq!(FailureCode::FlowAlertInteraction.number(), 417);
        assert_eq!(FailureCode::FlowExternalFailed.number(), 424);
        assert_eq!(FailureCode::FlowMaxRuntime.number(), 504);
        assert_eq!(FailureCode::FlowMaxSteps.number(), 508);
        assert_eq!(FailureCode::ParseMissingReference.number(), 404);
        assert_eq!(FailureCode::ParseVariable.number(), 450);
        assert_eq!(FailureCode::ParseFlowFile.number(), 460);
        assert_eq!(FailureCode::ParseDeviceConstraint.number(), 461);
        assert_eq!(FailureCode::AppInstallScriptNotFound.number(), 404);
        assert_eq!(FailureCode::AppInstallTimeout.number(), 408);
        assert_eq!(FailureCode::AppStateQueryFailed.number(), 502);
        assert_eq!(FailureCode::DeviceBootTimeout.number(), 408);
        assert_eq!(FailureCode::DeviceBusy.number(), 409);
        assert_eq!(FailureCode::DeviceCreateFailed.number(), 500);
        assert_eq!(FailureCode::DeviceWebviewComms.number(), 502);
        assert_eq!(FailureCode::DeviceRegistrationTimeout.number(), 504);
        assert_eq!(FailureCode::HostPortsExhausted.number(), 429);
    }

    // 8. extract_code returns the OUTERMOST tag when two are chained.
    #[test]
    fn extract_returns_outermost_tag() {
        let inner: anyhow::Result<()> =
            Err(anyhow!("inner cause")).code(FailureCode::FlowElementNotFound);
        let e = match inner {
            Err(e) => e,
            Ok(()) => panic!("expected Err"),
        };
        // Re-tag the already-coded error with a different, outer code.
        let outer = coded(FailureCode::FlowStepTimeout, e);
        assert_eq!(
            extract_code(&outer),
            Some(FailureCode::FlowStepTimeout),
            "extract_code SHALL return the outermost tag"
        );
    }

    // 9. coded flattens the inner message so {e:#} never double-prints the cause.
    #[test]
    fn coded_flattens_without_double_printing_cause() {
        let layered: anyhow::Result<()> = Err(anyhow!("root cause"));
        let layered = layered.map_err(|e| e.context("doing work"));
        let e = match layered {
            Err(e) => coded(FailureCode::FlowExternalFailed, e),
            Ok(()) => panic!("expected Err"),
        };
        let msg = clean_msg(&e);
        // The flattened message preserves the full chain exactly once.
        assert_eq!(msg, "doing work: root cause");
        assert_eq!(
            msg.matches("root cause").count(),
            1,
            "cause SHALL appear exactly once"
        );
        assert_eq!(extract_code(&e), Some(FailureCode::FlowExternalFailed));
    }

    // 11. CodeExt leaves Ok values untouched (no spurious tagging).
    #[test]
    fn code_ext_preserves_ok() {
        let r: anyhow::Result<i32> = Ok(7);
        let tagged = r.code(FailureCode::FlowExplicitFail);
        assert_eq!(tagged.ok(), Some(7), "Ok SHALL pass through untagged");
    }
}
