use std::path::{Path, PathBuf};
use std::time::Instant;

use std::sync::Arc;

use anyhow::Result;
use golem_devices::{DeviceInfo, DeviceState, Platform};
use golem_driver::android::AndroidDriver;
use golem_driver::ios::IosDriver;
use golem_driver::PlatformDriver;
use golem_orchestrator::{
    device_matches_slot, plan, CoverageGroup, DeviceSlot, FlowRun, InstallEntry,
};
use golem_parser::FlowFile;
use golem_report::{FlowReport, SuiteReport};
use golem_runner::capture::CaptureConfig;
use golem_runner::context::ExecutionContext;
use golem_runner::executor::execute_flow;
use golem_vars::VariableStore;

/// Recognise transient install-script errors that warrant a single
/// retry. The artifact has already been built at this point; the retry
/// runs the script with `install-only` so only the install step (e.g.
/// `simctl install`, `adb install`) is re-attempted.
///
/// Conservative match list — only patterns observed in real flake reports
/// where retrying actually helped. Adding patterns liberally would mask
/// genuine failures behind a 3s delay; better to fail fast on unknowns
/// and add the pattern explicitly when a new transient is identified.
fn is_transient_install_error(err: &str) -> bool {
    // CoreSimulator's IPC pipe occasionally crashes during install on
    // a freshly-booted iOS 26 sim. Format observed in stderr tail:
    //   "Mach error -308 - (ipc/mig) server died"
    //   "domain=NSMachErrorDomain, code=-308"
    // Match the canonical 308 token to catch both renderings.
    if err.contains("Mach error -308") || err.contains("NSMachErrorDomain, code=-308") {
        return true;
    }
    // adb's intermittent "device offline" race during emulator early boot.
    // Often clears within a couple seconds.
    if err.contains("error: device offline") || err.contains("error: device not found") {
        return true;
    }
    false
}

/// Format the human-readable target string used in install events:
/// `iPhone 16e (ios/v18/phone)`. Single-source so events emitted from
/// pre-install (`InstallStarted`/`Skipped`) and per-flow install paths
/// agree on every character.
fn format_install_target(device: &DeviceInfo, platform_str: &str) -> String {
    format!(
        "{} ({}/v{}/{})",
        device.name, platform_str, device.os_major, device.device_type
    )
}

/// Render a device's coverage axes — the human-readable axis-value list
/// that tells the user which tick boxes this run satisfied. Example:
/// `["ios", "v26", "tablet"]`. Populated onto `FlowReport.covered_axes`
/// so renderers can surface coverage without plumbing the slot through.
fn device_covered_axes(device: &DeviceInfo) -> Vec<String> {
    vec![
        device.platform.to_string(),
        format!("v{}", device.os_major),
        device.device_type.to_string(),
    ]
}

/// Execute-time coverage progress for one `CoverageGroup`.
///
/// - `ticked` — pool indices that at least one picked device has satisfied.
/// - `runs` — count of successful FlowRuns in the group (used by `One` /
///   any future JIT-N to cap runs independently of tick coverage).
///
/// The scheduler consults [`is_group_complete`] before each spawn; once
/// the stop condition is met, remaining group members short-circuit with
/// no FlowReport so they don't pollute suite results.
#[derive(Debug, Default)]
struct GroupProgress {
    ticked: std::collections::HashSet<usize>,
    runs: u32,
}

/// A group is done when either the run cap is hit or every pool box has
/// been ticked by a successful run. `max_runs = None` + empty pool → never
/// complete (defensive; the planner guards against empty pools).
fn is_group_complete(group: &CoverageGroup, progress: &GroupProgress) -> bool {
    if let Some(max) = group.max_runs {
        if progress.runs >= max {
            return true;
        }
    }
    if !group.boxes.is_empty() && progress.ticked.len() >= group.boxes.len() {
        return true;
    }
    false
}

/// Pool indices a device ticks — used to credit bonus coverage when a
/// picked device coincidentally satisfies other pool entries beyond the
/// ones the FlowRun pre-declared in `covers_boxes`.
fn pool_ticks_for_device(device: &DeviceInfo, group: &CoverageGroup) -> Vec<usize> {
    group
        .boxes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| if device_matches_slot(device, b) { Some(i) } else { None })
        .collect()
}

/// Configuration for a suite run.
pub struct SuiteConfig {
    /// Skip cleaning device state between flows.
    pub no_clean: bool,
    /// Skip teardown steps after each flow.
    pub no_teardown: bool,
    /// Keep device connections alive across flows.
    pub keep_devices: bool,
    /// Fixed random seed to use for all flows. When `None`, each flow
    /// gets an independent random seed.
    pub seed: Option<u64>,
    /// Force a specific platform, overriding flow device constraints.
    pub platform: Option<Platform>,
    /// Disable automatic performance capture.
    pub no_perf: bool,
    /// Show substep detail in human output.
    pub verbose: bool,
    /// Show driver-level diagnostics (WebKit/CDP).
    pub debug: bool,
    /// Whether to stream human-readable output to stderr.
    pub stream_human: bool,
    /// Start execution at this named block (skip earlier blocks).
    /// Assumes app is already in the correct state for that block.
    pub start: Option<String>,
    /// CLI-injected variables (--var KEY=VALUE).
    pub vars: Vec<(String, String)>,
    /// Root output directory for results, screenshots, recordings.
    pub output_dir: PathBuf,
    /// Disable all file output (screenshots, recordings, reports).
    pub no_results: bool,
    /// Project root directory (where install scripts are resolved from).
    pub project_root: PathBuf,
    /// Project-level app definitions from golem.toml `[[apps]]`. Flows
    /// inherit bundle/install_script/install_timeout_ms/devices from
    /// matching entries by name.
    pub project_apps: Vec<golem_parser::ProjectAppConfig>,
    /// CLI `--coverage` override. When Some, every flow's coverage
    /// strategy is forced to this value regardless of `[flow.options]`.
    /// Useful for quick smoke runs: `--coverage one`.
    pub coverage_override: Option<golem_parser::CoverageStrategy>,
    /// `--rebuild`: bypass the persistent install cache for this run
    /// (rebuild + reinstall every (device, bundle)). The cache is still
    /// written after a successful install.
    pub rebuild: bool,
    /// `--no-build`: skip build+install entirely. Devices that already
    /// have the bundle installed run flows; devices that don't fail
    /// loudly. The cache is untouched.
    pub no_build: bool,
}

impl Default for SuiteConfig {
    fn default() -> Self {
        Self {
            no_clean: false,
            no_teardown: false,
            keep_devices: false,
            seed: None,
            platform: None,
            no_perf: false,
            verbose: false,
            debug: false,
            stream_human: false,
            start: None,
            vars: Vec::new(),
            output_dir: PathBuf::from(".golem/results"),
            no_results: false,
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            project_apps: Vec::new(),
            coverage_override: None,
            rebuild: false,
            no_build: false,
        }
    }
}

/// Orchestrates the execution of a suite of test flows.
pub struct SuiteRunner {
    pub config: SuiteConfig,
    /// Shared resource manager for device allocation across flows.
    pub resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    /// Optional external event sender for forwarding events (e.g. to orchestrator client).
    pub event_forwarder: Option<golem_events::channel::EventSender>,
    /// Suite-level install cache, shared by all flows so a given
    /// `(device, bundle)` install script runs at most once per suite.
    pub install_cache: golem_runner::installer::InstallCache,
    /// Install matrix computed at `run_suite` start by `golem_orchestrator::plan()`.
    /// Only apps referenced by some flow appear here. Consumed by pre-install
    /// in the per-device setup loop. Empty until `run_suite` has been called.
    pub install_matrix: Arc<Vec<InstallEntry>>,
    /// Parsed-flow paths in the order the plan saw them — index is the
    /// `flow_idx` used by `flow_runs`. Consumed by per-flow device
    /// selection so execute can look up each FlowRun's slot requirements
    /// by flow path. Empty outside of `run_suite` (direct
    /// `run_single_flow_with_resources` test harnesses see an empty Arc
    /// and fall back to platform-only device filtering).
    pub flow_paths: Arc<Vec<PathBuf>>,
    /// FlowRuns emitted by `plan()`. Each carries `DeviceSlot`
    /// requirements the scheduler uses to pick a matching free device.
    /// Co-populated with `flow_paths`.
    pub flow_runs: Arc<Vec<FlowRun>>,
    /// One-shot `SuitePlanned` event cache. Populated in `run_suite` from
    /// the Plan output (only when `--verbose` is on) and `take()`-ed by
    /// whichever execute path first attaches subscribers (multi-flow:
    /// `suite_tx`; single-flow: per-flow `event_tx`).
    ///
    /// Contract: set once per `run_suite` call, consumed at most once. After
    /// consumption subsequent emit paths see `None`. Callers that invoke
    /// `run_single_flow_with_resources` directly (test harnesses, future
    /// scheduler adapters) will not see the plan summary unless they
    /// populate `plan_event` themselves before the call.
    pub plan_event: Option<golem_events::EventKind>,
}

impl SuiteRunner {
    pub fn new(config: SuiteConfig) -> Self {
        Self {
            config,
            resource_mgr: std::sync::Arc::new(
                golem_devices::resource_manager::ResourceManager::new(
                    golem_devices::concurrency::ConcurrencyConfig::default(),
                ),
            ),
            event_forwarder: None,
            install_cache: golem_runner::installer::InstallCache::new(),
            install_matrix: Arc::new(Vec::new()),
            flow_paths: Arc::new(Vec::new()),
            flow_runs: Arc::new(Vec::new()),
            plan_event: None,
        }
    }

    /// Create a runner with a shared ResourceManager and InstallCache
    /// (for orchestrator mode). The cache survives across submits to the
    /// same `OrchestratorServer`, so a second submit can reuse a prior
    /// submit's install work on the same device — the actual installs all
    /// happen in the server process, so clients pick up the hits
    /// transparently. Cache lifetime = server process lifetime.
    pub fn with_resource_manager(
        config: SuiteConfig,
        resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
        install_cache: golem_runner::installer::InstallCache,
    ) -> Self {
        Self {
            config,
            resource_mgr,
            event_forwarder: None,
            install_cache,
            install_matrix: Arc::new(Vec::new()),
            flow_paths: Arc::new(Vec::new()),
            flow_runs: Arc::new(Vec::new()),
            plan_event: None,
        }
    }

    /// Run a suite of flow files and return aggregated results.
    ///
    /// JIT FlowRun scheduler: the Plan phase parses every flow, expands
    /// device coverage, and emits a list of `FlowRun` units. Each FlowRun
    /// is the smallest executable unit (one coverage point × one flow —
    /// single-slot usually, multi-slot for chat-test coordination). The
    /// scheduler spawns one worker task per FlowRun; the shared
    /// `ResourceManager` throttles concurrent device allocations by RAM +
    /// max-concurrency. Workers that can't get a device wait until one
    /// frees.
    ///
    /// Setup is lazy-per-FlowRun rather than global-phased: each worker
    /// does its own `find_available_device` → JIT install (reuses
    /// `install_cache` to avoid repeating work) → companion → allocate.
    /// A FlowRun for iOS can therefore start before an unrelated Android
    /// install finishes.
    ///
    /// Parse failures from Plan become failed `FlowReport`s (not a hard
    /// suite error) — one bad flow file does not abort the rest of the
    /// suite.
    pub async fn run_suite(&mut self, flow_paths: &[PathBuf]) -> Result<SuiteReport> {
        let start = Instant::now();

        // Plan phase: parse + merge + expand coverage + build install matrix.
        // Only apps referenced by some flow end up in the matrix — apps declared
        // in `golem.toml [[apps]]` that no flow uses are dropped entirely.
        let parsed = plan(
            flow_paths,
            &self.config.project_apps,
            &self.config.project_root,
            self.config.platform,
            self.config.coverage_override,
        )
        .await?;

        // Build the SuitePlanned event under --verbose. Emitted via the
        // event channel once it's set up.
        if self.config.verbose {
            self.plan_event = Some(build_suite_planned_event(&parsed));
        }

        self.install_matrix = Arc::new(parsed.install_matrix.clone());
        self.flow_paths = Arc::new(flow_paths.to_vec());
        self.flow_runs = Arc::new(parsed.flow_runs.clone());

        // Persistent install cache: load + fingerprint compute only when
        // there's at least one install entry to gate. Empty matrices
        // (test harnesses, flows with no `install_script`) skip the work
        // entirely so suite startup stays fast.
        let needs_cache = !parsed.install_matrix.is_empty() && !self.config.no_build;
        if needs_cache {
            let cache_path = self.config.project_root.join(".golem/install-cache.json");
            if let Err(e) = self.install_cache.load_persistent(cache_path).await {
                eprintln!("  [install] cache load failed ({e}) — continuing with empty cache");
            }
        }
        let fingerprint = Arc::new(if needs_cache {
            golem_runner::fingerprint::Fingerprint::compute(&self.config.project_root)
        } else {
            golem_runner::fingerprint::Fingerprint::None
        });
        if self.config.verbose && needs_cache {
            eprintln!("  [install] source fingerprint: {}", fingerprint.short_label());
        }

        // Suite-level event channel — ONE sink for every FlowRun worker,
        // every setup-phase emitter, and the plan summary. Previously the
        // single-flow path created its own channel and the multi-flow path
        // another; JIT uses one so output ordering is deterministic.
        let resource_mgr = self.resource_mgr.clone();
        let (suite_tx, suite_rx) = golem_events::channel::event_channel();
        let verbose = self.config.verbose;
        let debug = self.config.debug;
        let stream_human_enabled = self.config.stream_human;
        if debug {
            golem_driver::set_debug(true);
        }

        // multi_device=true gives device prefixes — right whenever we have
        // more than one device in play (coverage fan-out, chat coordination,
        // or multi-flow).
        let multi_device = parsed.flow_runs.len() > 1
            || parsed.flow_runs.iter().any(|r| r.slots.len() > 1);

        let human_handle = if stream_human_enabled {
            let human_rx = suite_rx.subscribe();
            Some(tokio::spawn(async move {
                golem_report::stream::stream_human(human_rx, verbose, multi_device, debug).await;
            }))
        } else {
            None
        };

        let accumulator = std::sync::Arc::new(tokio::sync::Mutex::new(
            golem_report::accumulator::ReportAccumulator::new(),
        ));
        let acc_clone = accumulator.clone();
        let acc_rx = suite_rx.subscribe();
        let acc_handle = tokio::spawn(async move {
            golem_report::accumulator::accumulate_events(acc_rx, &acc_clone).await;
        });

        // Forward events to external sender (orchestrator client) if present.
        let fwd_handle = if let Some(ref fwd) = self.event_forwarder {
            let fwd_rx = suite_rx.subscribe();
            let fwd_tx = fwd.clone();
            Some(tokio::spawn(async move {
                let mut rx = fwd_rx;
                while let Ok(event) = rx.recv().await {
                    fwd_tx.emit(event.device_id.clone(), event.kind.clone());
                }
            }))
        } else {
            None
        };

        // Emit the Plan summary (if any) now that subscribers are attached.
        if let Some(event) = self.plan_event.take() {
            suite_tx.emit(golem_events::DeviceId("suite".into()), event);
        }

        drop(suite_rx);

        // Start the registration server once, share across every worker.
        // Companions register here; workers look up the port post-registration.
        let (reg_state, _reg_rx) = crate::registration::RegistrationState::new();
        let reg_port = crate::registration::start_registration_server(reg_state.clone())
            .await
            .unwrap_or(0);

        // Parse failures become failed flow reports immediately — they don't
        // need a worker, they just need to appear in the output stream so
        // users see every flow file they asked to run.
        let mut flow_reports: Vec<FlowReport> = Vec::new();
        for pf in &parsed.parse_failures {
            suite_tx.emit(
                golem_events::DeviceId("suite".into()),
                golem_events::EventKind::FlowParseFailed {
                    path: pf.path.display().to_string(),
                    error: pf.error.clone(),
                },
            );
            let flow_name = pf
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            flow_reports.push(FlowReport {
                flow_name,
                success: false,
                step_results: Vec::new(),
                warnings: vec![format!("Parse/mixin error: {}", pf.error)],
                duration_ms: 0,
                seed: self.config.seed,
                screenshot_path: None,
                device_name: None,
                os_major: None,
                perf_snapshots: vec![],
                skipped_reason: None,
                covered_axes: Vec::new(),
                started_at: None,
                finished_at: None,
            });
        }

        // Execute-time coverage tracker for `One` / `Smart` groups. The
        // scheduler consults this before each spawn: once a group's stop
        // condition is met (max_runs hit or all pool boxes ticked), every
        // remaining member short-circuits. Built once per suite.
        let coverage_groups = Arc::new(parsed.coverage_groups.clone());
        let coverage_progress: Arc<
            tokio::sync::Mutex<std::collections::HashMap<usize, GroupProgress>>,
        > = {
            let mut init = std::collections::HashMap::new();
            for idx in 0..parsed.coverage_groups.len() {
                init.insert(idx, GroupProgress::default());
            }
            Arc::new(tokio::sync::Mutex::new(init))
        };

        // Per-group dispatch locks: groups with a `max_runs` cap (today:
        // `one`) need sibling FlowRuns to run sequentially, otherwise
        // parallel spawns race past the gate and every group member
        // installs before the first success can update progress. Smart
        // groups (`max_runs = None`) don't need this — parallel is
        // faster since Smart wants every pool box ticked.
        let dispatch_locks: std::collections::HashMap<usize, Arc<tokio::sync::Mutex<()>>> =
            parsed
                .coverage_groups
                .iter()
                .enumerate()
                .filter(|(_, g)| g.max_runs.is_some())
                .map(|(idx, _)| (idx, Arc::new(tokio::sync::Mutex::new(()))))
                .collect();

        // Spawn one worker per FlowRun. The ResourceManager gates how many
        // run concurrently: `try_allocate` fails when RAM/concurrency caps
        // would be exceeded, and workers retry on a 2s backoff.
        let mut handles = Vec::new();
        for run in parsed.flow_runs.iter() {
            let Some(pf) = parsed.flows.get(run.flow_idx) else {
                continue;
            };
            let flow = pf.flow.clone();
            let path = pf.path.clone();
            let slots = run.slots.clone();

            let rm = resource_mgr.clone();
            let install_cache = self.install_cache.clone();
            let install_matrix = self.install_matrix.clone();
            let tx = suite_tx.clone();
            let reg_state = reg_state.clone();
            let seed = self.config.seed;
            let start_block = self.config.start.clone();
            let cli_vars = self.config.vars.clone();
            let output_dir = self.config.output_dir.clone();
            let no_results = self.config.no_results;
            let no_perf = self.config.no_perf;
            let debug = self.config.debug;
            let project_root = self.config.project_root.clone();
            let fingerprint = fingerprint.clone();
            let rebuild = self.config.rebuild;
            let no_build = self.config.no_build;
            let coverage_groups_c = coverage_groups.clone();
            let coverage_progress_c = coverage_progress.clone();
            let coverage_group_idx = run.coverage_group;
            let covers_boxes = run.covers_boxes.clone();
            let dispatch_lock = run
                .coverage_group
                .and_then(|gi| dispatch_locks.get(&gi).cloned());

            handles.push(tokio::spawn(async move {
                let reports = execute_flow_run(
                    path,
                    flow,
                    slots,
                    rm,
                    install_cache,
                    install_matrix,
                    tx,
                    reg_port,
                    reg_state,
                    FlowRunConfig {
                        seed,
                        start_block,
                        cli_vars,
                        output_dir,
                        no_results,
                        no_perf,
                        debug,
                        project_root,
                        fingerprint,
                        rebuild,
                        no_build,
                    },
                    CoverageCtx {
                        groups: coverage_groups_c,
                        progress: coverage_progress_c,
                        group_idx: coverage_group_idx,
                        covers_boxes,
                        dispatch_lock,
                    },
                )
                .await;
                (reports, coverage_group_idx)
            }));
        }

        // Collect per-FlowRun reports alongside their coverage-group idx so
        // the post-pass below can reclassify failed peers in `one`-strategy
        // groups whose goal was already met by another member.
        let mut reports_with_group: Vec<(FlowReport, Option<usize>)> = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((reports, group_idx)) => {
                    for r in reports {
                        reports_with_group.push((r, group_idx));
                    }
                }
                Err(e) => {
                    reports_with_group.push((FlowReport {
                        flow_name: "unknown".to_string(),
                        success: false,
                        step_results: Vec::new(),
                        warnings: vec![format!("Task panicked: {e}")],
                        duration_ms: 0,
                        seed: self.config.seed,
                        screenshot_path: None,
                        device_name: None,
                        os_major: None,
                        perf_snapshots: vec![],
                        skipped_reason: None,
                        covered_axes: Vec::new(),
                        started_at: None,
                        finished_at: None,
                    }, None));
                }
            }
        }

        // Skip-reclassify pass: in `coverage = "one"` groups, any failed
        // FlowRun whose peer already satisfied the group's max_runs cap is
        // reclassified as skipped — the user explicitly asked for "stop
        // after one success", so peer failures shouldn't pollute the
        // summary or fail the suite. Smart groups (max_runs=None) keep
        // failures visible since Smart's goal is *full* coverage.
        {
            let final_progress = coverage_progress.lock().await;
            for (report, group_idx) in &mut reports_with_group {
                if report.success {
                    continue;
                }
                let Some(gi) = group_idx else { continue };
                let Some(group) = coverage_groups.get(*gi) else { continue };
                if group.max_runs.is_none() {
                    continue;
                }
                let Some(p) = final_progress.get(gi) else { continue };
                if p.runs >= 1 {
                    report.success = true;
                    report.skipped_reason =
                        Some("coverage group satisfied by peer run".to_string());
                }
            }
        }

        flow_reports.extend(reports_with_group.into_iter().map(|(r, _)| r));

        // Emit suite summary and close suite channel.
        let passed = flow_reports.iter().filter(|r| r.is_passed()).count();
        let failed = flow_reports.iter().filter(|r| r.is_failed()).count();
        let skipped = flow_reports.iter().filter(|r| r.is_skipped()).count();
        suite_tx.emit(
            golem_events::DeviceId("suite".into()),
            golem_events::EventKind::SuiteFinished {
                duration_ms: start.elapsed().as_millis() as u64,
                passed,
                failed,
                skipped,
            },
        );
        drop(suite_tx);
        if let Some(h) = human_handle { let _ = h.await; }
        let _ = acc_handle.await;
        if let Some(h) = fwd_handle { let _ = h.await; }

        // Merge step data from suite-level accumulator into flow reports.
        let acc_report = {
            let taken = std::mem::replace(
                &mut *accumulator.lock().await,
                golem_report::accumulator::ReportAccumulator::new(),
            );
            taken.into_suite_report()
        };
        for report in &mut flow_reports {
            if let Some(acc_flow) = acc_report.flows.iter().find(|f| {
                f.device_name.as_deref() == report.device_name.as_deref()
                    && f.flow_name == report.flow_name
            }) {
                // Accumulator built its own FlowReport from live events —
                // prefer its wall-clock timestamps and os_major since
                // run_flow_on_device doesn't capture them directly on the
                // returned FlowReport.
                if report.started_at.is_none() {
                    report.started_at = acc_flow.started_at.clone();
                }
                if report.finished_at.is_none() {
                    report.finished_at = acc_flow.finished_at.clone();
                }
                if report.os_major.is_none() {
                    report.os_major = acc_flow.os_major;
                }
                if report.step_results.is_empty() && !acc_flow.step_results.is_empty() {
                    report.step_results = acc_flow.step_results.iter().map(|s| {
                        golem_report::StepReport {
                            global_step_index: s.global_step_index,
                            block_name: s.block_name.clone(),
                            block_iteration: s.block_iteration,
                            step_index_in_block: s.step_index_in_block,
                            action: s.action.clone(),
                            target: s.target.clone(),
                            outcome: match &s.outcome {
                                golem_report::StepOutcome::Success => golem_report::StepOutcome::Success,
                                golem_report::StepOutcome::Warning(m) => golem_report::StepOutcome::Warning(m.clone()),
                                golem_report::StepOutcome::Failed(m) => golem_report::StepOutcome::Failed(m.clone()),
                                golem_report::StepOutcome::Skipped => golem_report::StepOutcome::Skipped,
                            },
                            duration_ms: s.duration_ms,
                            retry_count: s.retry_count,
                            screenshot_path: s.screenshot_path.clone(),
                            substeps: s.substeps.clone(),
                            tree_stats: s.tree_stats,
                            started_at: s.started_at.clone(),
                            finished_at: s.finished_at.clone(),
                        }
                    }).collect();
                }
            }
        }

        Ok(SuiteReport {
            flows: flow_reports,
            installs: acc_report.installs,
            total_duration_ms: start.elapsed().as_millis() as u64,
            started_at: acc_report.started_at,
            finished_at: acc_report.finished_at,
        })
    }

}

/// Config bundle handed to `execute_flow_run` workers. Narrower than
/// `SuiteConfig` — only the fields a FlowRun worker actually needs.
struct FlowRunConfig {
    seed: Option<u64>,
    start_block: Option<String>,
    cli_vars: Vec<(String, String)>,
    output_dir: PathBuf,
    no_results: bool,
    no_perf: bool,
    debug: bool,
    project_root: PathBuf,
    /// Source-tree fingerprint computed once at suite start. Used by the
    /// persistent install cache to decide whether to skip build+install.
    fingerprint: Arc<golem_runner::fingerprint::Fingerprint>,
    /// CLI `--rebuild`: bypass cache read for this run; rebuild + write.
    rebuild: bool,
    /// CLI `--no-build`: skip build+install if device already has the bundle.
    no_build: bool,
}

/// Build a synthetic `FlowReport` for a FlowRun short-circuited by the
/// coverage gate. No device was acquired, no steps ran. Success=true so
/// the suite exit code isn't polluted; skipped_reason carries the cause
/// so renderers surface it as `SKIP`. `covered_axes` is derived from the
/// first slot's shape — the axes this FlowRun *would* have ticked had it
/// run. Gives users visibility into which group members the scheduler
/// spared.
fn coverage_skip_report(
    flow_name: String,
    slots: &[DeviceSlot],
    seed: Option<u64>,
    start: Instant,
) -> FlowReport {
    let (device_name, covered_axes) = slots
        .first()
        .map(|s| {
            let label = golem_orchestrator::shape_label(s);
            let axes: Vec<String> = label.split('/').map(|p| p.to_string()).collect();
            (Some(label), axes)
        })
        .unwrap_or((None, Vec::new()));
    FlowReport {
        flow_name,
        success: true,
        step_results: Vec::new(),
        warnings: Vec::new(),
        duration_ms: start.elapsed().as_millis() as u64,
        seed,
        screenshot_path: None,
        device_name,
        os_major: None,
        perf_snapshots: Vec::new(),
        skipped_reason: Some("coverage group satisfied by peer run".to_string()),
        covered_axes,
        started_at: None,
        finished_at: None,
    }
}

/// Coverage-group context plumbed into every FlowRun worker. `group_idx`
/// is `None` for `Min` / `Full` (no group gating); `Some(i)` points at
/// `groups[i]`, and `covers_boxes` lists pool indices this run will tick
/// on success. `progress` is the shared live tracker.
///
/// `dispatch_lock` is `Some` only for groups with `max_runs = Some(_)`
/// (today: `coverage = "one"`). Every member of such a group shares one
/// `Mutex`; the worker acquires it before the pre-setup gate and drops it
/// after progress is updated. Siblings block until the first run fully
/// completes, so the second+ always sees an accurate gate state — no
/// parallel double-install on the losing devices. Groups with
/// `max_runs = None` (today: `smart`) get `None` here: parallel runs are
/// useful because Smart's stop condition is "every pool box ticked", which
/// is faster with concurrent progress.
struct CoverageCtx {
    groups: Arc<Vec<CoverageGroup>>,
    progress: Arc<tokio::sync::Mutex<std::collections::HashMap<usize, GroupProgress>>>,
    group_idx: Option<usize>,
    covers_boxes: Vec<usize>,
    dispatch_lock: Option<Arc<tokio::sync::Mutex<()>>>,
}

/// Execute a single `FlowRun`: set up each slot (device + preinstall +
/// companion + allocation), then spawn per-slot runners sharing a
/// `FailureBarrier`. Releases every device back to the `ResourceManager`
/// when the run finishes. Returns one `FlowReport` per slot that produced
/// a result.
///
/// This is the unit the JIT scheduler queues. Each worker calls this
/// directly on its assigned FlowRun; no further platform inference is
/// needed because the slots already carry platform + os_version +
/// device_type + other requirements from the Plan phase.
#[allow(clippy::too_many_arguments)]
async fn execute_flow_run(
    path: PathBuf,
    flow: FlowFile,
    slots: Vec<DeviceSlot>,
    resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: golem_runner::installer::InstallCache,
    install_matrix: Arc<Vec<InstallEntry>>,
    event_tx: golem_events::channel::EventSender,
    reg_port: u16,
    reg_state: crate::registration::RegistrationState,
    cfg: FlowRunConfig,
    coverage: CoverageCtx,
) -> Vec<FlowReport> {
    let start = Instant::now();
    let flow_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Serialise within the coverage group when it has a run cap (today:
    // `coverage = "one"`). Hold the lock for the whole body so siblings
    // wait until this run's progress update is visible. Parallel groups
    // (`smart`, no group) pass `None` → no blocking.
    let _dispatch_guard = if let Some(lock) = coverage.dispatch_lock.clone() {
        Some(lock.lock_owned().await)
    } else {
        None
    };

    // Pre-setup coverage gate: if another FlowRun in this group already
    // satisfied the stop condition, skip entirely — don't acquire a
    // device, don't preinstall, don't start a companion. Emit a
    // `FlowSkipped` event (for live stream rendering) plus a synthetic
    // `SKIP` report (for summary counts + final output), so the user
    // sees the group member was deliberately spared — not silently
    // vanishing — while exit code stays 0.
    if coverage_group_done(&coverage).await {
        let reason = "coverage group satisfied by peer run".to_string();
        event_tx.emit(
            golem_events::DeviceId("suite".into()),
            golem_events::EventKind::FlowSkipped {
                flow_name: flow_name.clone(),
                reason: reason.clone(),
            },
        );
        return vec![coverage_skip_report(flow_name, &slots, cfg.seed, start)];
    }

    let create_if_missing = flow
        .flow
        .options
        .as_ref()
        .and_then(|o| o.create_if_missing)
        .unwrap_or(false);

    // Per-slot setup: find device, preinstall apps for that slot's
    // platform, ensure companion, allocate. Setup runs sequentially
    // across slots — typical flow has one slot; chat-test flows have 2
    // slots on different platforms so sequential setup is fine
    // (installs serialize on `project_lock` anyway).
    let mut device_setups: Vec<(DeviceInfo, u16)> = Vec::new();
    for slot in &slots {
        match setup_slot(
            slot,
            &resource_mgr,
            &install_cache,
            &install_matrix,
            &event_tx,
            reg_port,
            &reg_state,
            create_if_missing,
            &cfg.project_root,
            cfg.debug,
            &cfg.fingerprint,
            cfg.rebuild,
            cfg.no_build,
        )
        .await
        {
            Ok((device, port)) => device_setups.push((device, port)),
            Err(e) => {
                event_tx.emit(
                    golem_events::DeviceId("suite".into()),
                    golem_events::EventKind::SlotSetupFailed {
                        slot_label: golem_orchestrator::describe_slot(slot),
                        reason: format!("{e:#}"),
                    },
                );
            }
        }
    }

    if device_setups.is_empty() {
        return vec![FlowReport {
            flow_name,
            success: false,
            step_results: Vec::new(),
            warnings: vec!["No devices available for any target platform".to_string()],
            duration_ms: start.elapsed().as_millis() as u64,
            seed: cfg.seed,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            started_at: None,
            finished_at: None,
        }];
    }

    let allocated_udids: Vec<String> = device_setups
        .iter()
        .map(|(d, _)| d.udid.clone())
        .collect();

    // Post-setup coverage gate: a concurrent group member may have
    // succeeded while we were booting / installing. Release our devices
    // and emit a SKIP — the setup work is wasted, but the flow itself
    // didn't run and the user sees why.
    if coverage_group_done(&coverage).await {
        for udid in &allocated_udids {
            resource_mgr.release(udid);
        }
        let reason = "coverage group satisfied by peer run".to_string();
        event_tx.emit(
            golem_events::DeviceId("suite".into()),
            golem_events::EventKind::FlowSkipped {
                flow_name: flow_name.clone(),
                reason: reason.clone(),
            },
        );
        return vec![coverage_skip_report(flow_name, &slots, cfg.seed, start)];
    }

    // Clone the picked devices so we can compute bonus ticks after the
    // flow finishes (`device_setups` is moved into the spawn loop below).
    let picked_devices: Vec<DeviceInfo> =
        device_setups.iter().map(|(d, _)| d.clone()).collect();

    // Per-FlowRun barrier: a device failing at step N aborts the other
    // slot(s) at step ≥ N. MUST stay per-FlowRun — step counts only compare
    // within one execution. See `golem-runner/src/barrier.rs`.
    let barrier = golem_runner::barrier::FailureBarrier::new();
    let flow_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut handles = Vec::new();
    for (device, port) in device_setups {
        let platform = device.platform;
        let flow_c = flow.clone();
        let flow_name_c = flow_name.clone();
        let flow_dir_c = flow_dir.clone();
        let barrier_c = barrier.clone();
        let tx_c = event_tx.clone();
        let install_cache_c = install_cache.clone();
        let project_root_c = cfg.project_root.clone();
        let seed = cfg.seed;
        let start_block = cfg.start_block.clone();
        let cli_vars = cfg.cli_vars.clone();
        let output_dir = cfg.output_dir.clone();
        let no_results = cfg.no_results;
        let no_perf = cfg.no_perf;
        handles.push(tokio::spawn(async move {
            run_flow_on_device(
                flow_c,
                flow_name_c,
                flow_dir_c,
                device,
                platform,
                port,
                seed,
                start_block,
                cli_vars,
                output_dir,
                no_results,
                install_cache_c,
                project_root_c,
                barrier_c,
                no_perf,
                Some(tx_c),
            )
            .await
        }));
    }

    let mut reports = Vec::new();
    for h in handles {
        match h.await {
            Ok(r) => reports.push(r),
            Err(e) => {
                reports.push(FlowReport {
                    flow_name: flow_name.clone(),
                    success: false,
                    step_results: Vec::new(),
                    warnings: vec![format!("Task panicked: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: cfg.seed,
                    screenshot_path: None,
                    device_name: None,
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    started_at: None,
                    finished_at: None,
                });
            }
        }
    }

    for udid in &allocated_udids {
        resource_mgr.release(udid);
    }

    // If this run belongs to a coverage group and produced at least one
    // success, update the shared tracker: record the pre-declared
    // `covers_boxes` plus any bonus pool entries the picked devices
    // happen to tick. The next group member will see the updated
    // progress before it spawns.
    if let Some(gi) = coverage.group_idx {
        let any_success = reports.iter().any(|r| r.success);
        if any_success {
            let mut progress = coverage.progress.lock().await;
            if let (Some(group), Some(p)) =
                (coverage.groups.get(gi), progress.get_mut(&gi))
            {
                p.runs += 1;
                for i in &coverage.covers_boxes {
                    p.ticked.insert(*i);
                }
                for d in &picked_devices {
                    for i in pool_ticks_for_device(d, group) {
                        p.ticked.insert(i);
                    }
                }
            }
        }
    }

    reports
}

/// Check whether the coverage group this FlowRun belongs to is already
/// complete. Returns `false` for runs with no group (`Min`, `Full`).
async fn coverage_group_done(coverage: &CoverageCtx) -> bool {
    let Some(gi) = coverage.group_idx else {
        return false;
    };
    let progress = coverage.progress.lock().await;
    match (coverage.groups.get(gi), progress.get(&gi)) {
        (Some(group), Some(p)) => is_group_complete(group, p),
        _ => false,
    }
}

/// Prepare one slot for flow execution:
/// 1. Pick a matching free device (auto-boot a shutdown one if the slot
///    has no booted match — single-shot, not boot-on-demand).
/// 2. Run the install matrix entries applicable to this device + platform.
/// 3. Find or spawn a companion for the device.
/// 4. Wait until `ResourceManager` lets us allocate the device (RAM +
///    max-concurrency cap).
/// 5. Health-check the companion.
///
/// On any step's failure, releases anything we allocated and returns the
/// error so the caller can skip the slot.
#[allow(clippy::too_many_arguments)]
async fn setup_slot(
    slot: &DeviceSlot,
    resource_mgr: &std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: &golem_runner::installer::InstallCache,
    install_matrix: &[InstallEntry],
    event_tx: &golem_events::channel::EventSender,
    reg_port: u16,
    reg_state: &crate::registration::RegistrationState,
    create_if_missing: bool,
    project_root: &Path,
    debug: bool,
    fingerprint: &golem_runner::fingerprint::Fingerprint,
    rebuild: bool,
    no_build: bool,
) -> Result<(DeviceInfo, u16)> {
    let device = find_available_device(
        slot.platform,
        Some(slot),
        resource_mgr,
        create_if_missing,
        event_tx,
        Some(install_cache),
        install_matrix,
    ).await?;
    let platform = device.platform;

    if debug {
        eprintln!("  Platform: {platform}");
    }

    preinstall_for_device_scoped(
        &device,
        platform,
        install_matrix,
        install_cache,
        event_tx,
        project_root,
        fingerprint,
        rebuild,
        no_build,
    )
    .await;

    // Reuse an existing healthy companion if one's already bound to a port
    // we can detect, otherwise spawn a fresh one via the registration path.
    let existing_port = find_or_allocate_port(&device, platform).await.ok();
    let reused = if let Some(p) = existing_port {
        let client = golem_driver::common::CompanionClient::new(p);
        if let Ok(health) = client.check_health().await {
            if health.version == env!("CARGO_PKG_VERSION") {
                Some(p)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let port = match reused {
        Some(p) => p,
        None => ensure_companion_with_reg(&device, platform, reg_port, reg_state, event_tx).await?,
    };

    // Wait for a free allocation slot. The cap here is RAM + concurrency
    // from `ConcurrencyConfig`; if we're at the limit another FlowRun's
    // device release will unblock us. 20-min deadline is a safety net, not
    // an expected path.
    let alloc_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1200);
    let mut emitted_waiting = false;
    let device_label = format!("{platform}/{}", device.name);
    loop {
        match resource_mgr.try_allocate(&device, port) {
            Ok(()) => break,
            Err(e) => {
                if tokio::time::Instant::now() >= alloc_deadline {
                    anyhow::bail!("timed out waiting for resources: {e:#}");
                }
                if !emitted_waiting {
                    event_tx.emit(
                        golem_events::DeviceId(device_label.clone()),
                        golem_events::EventKind::ResourcesWaiting {
                            platform: platform.to_string(),
                        },
                    );
                    emitted_waiting = true;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }

    let client = golem_driver::common::CompanionClient::new(port);
    match client.check_health().await {
        Ok(health) => {
            event_tx.emit(
                golem_events::DeviceId(device_label.clone()),
                golem_events::EventKind::CompanionReady {
                    platform: health.platform.clone(),
                    version: health.version.clone(),
                    device_name: health.device_name.clone(),
                    os_version: health.os_version.clone(),
                },
            );
            Ok((device, port))
        }
        Err(e) => {
            resource_mgr.release(&device.udid);
            anyhow::bail!("companion health check failed: {e:#}")
        }
    }
}

/// Install every `InstallEntry` from the suite's install matrix that is
/// applicable to `(device, platform)`. Runs BEFORE companion usage so that
/// `simctl install` / `adb install` can't tear down an xctest /
/// instrumentation session mid-suite.
///
/// An entry is applicable when its `platform` matches, the
/// `(device.udid, entry.bundle_id)` pair is not already cached, and the
/// entry's `device_constraints` (from `[[flow.apps.devices]]`) don't
/// exclude the device. The device resolver normally prevents mismatches
/// upstream; this check is a safety net.
///
/// Outcomes are written to `install_cache`; a later per-flow install
/// check sees `Succeeded` / `FailedScript` and skips re-running or skips
/// the flow respectively.
#[allow(clippy::too_many_arguments)]
async fn preinstall_for_device_scoped(
    device: &DeviceInfo,
    platform: Platform,
    install_matrix: &[InstallEntry],
    install_cache: &golem_runner::installer::InstallCache,
    event_tx: &golem_events::channel::EventSender,
    project_root: &Path,
    fingerprint: &golem_runner::fingerprint::Fingerprint,
    rebuild: bool,
    no_build: bool,
) {
    let platform_str = platform.to_string();
    // Use the same device_label format as per-flow emission so
    // stream_human's circled-number mapping stays stable across
    // preinstall and flow events for the same physical device.
    let device_label = format!("{platform}/{}", device.name);
    let emitter = golem_events::emitter::DeviceEmitter::new(
        event_tx.clone(),
        golem_events::DeviceId(device_label),
    );
    for entry in install_matrix.iter() {
        if entry.platform != platform {
            continue;
        }
        if !device_matches_entry_constraints(device, entry) {
            continue;
        }

        let target = format_install_target(device, &platform_str);

        // --no-build path: trust the device. If the bundle is installed,
        // mark Succeeded and move on. If not, mark FailedScript with an
        // actionable message — the per-flow install check will turn that
        // into a loud flow failure.
        if no_build {
            let info = golem_runner::installed_state::query(device, &entry.bundle_id).await;
            let key = (device.udid.clone(), entry.bundle_id.clone());
            if info.installed {
                emitter.emit(golem_events::EventKind::InstallSkipped {
                    app_name: entry.app_name.clone(),
                    bundle_id: entry.bundle_id.clone(),
                    target: target.clone(),
                    reason: "no-build: bundle present on device".to_string(),
                });
                install_cache
                    .set(key, golem_runner::installer::InstallOutcome::Succeeded)
                    .await;
            } else {
                let msg = format!(
                    "--no-build: {} not installed on {}; drop --no-build or install manually",
                    entry.bundle_id, device.name
                );
                install_cache
                    .set(
                        key,
                        golem_runner::installer::InstallOutcome::FailedScript(msg),
                    )
                    .await;
            }
            continue;
        }

        // Strict cache gate (default): only consult on cache reads when
        // not --rebuild. `rebuild=true` skips the read but still writes
        // after a successful build, so the next run benefits.
        if !rebuild {
            match evaluate_cache_gates(install_cache, device, &entry.bundle_id, fingerprint).await {
                CacheVerdict::Hit { label: _ } => {
                    // On hit, the label (e.g. `git:abc1234`) is just the
                    // source-fingerprint identity — informational at best,
                    // not actionable. Misses are where the user needs
                    // detail; on a hit, just say so and move on.
                    emitter.emit(golem_events::EventKind::InstallSkipped {
                        app_name: entry.app_name.clone(),
                        bundle_id: entry.bundle_id.clone(),
                        target: target.clone(),
                        reason: "cache hit".into(),
                    });
                    install_cache
                        .set(
                            (device.udid.clone(), entry.bundle_id.clone()),
                            golem_runner::installer::InstallOutcome::Succeeded,
                        )
                        .await;
                    continue;
                }
                CacheVerdict::Miss { reason } => {
                    // Skip the noisy "no prior cache entry" message — that's
                    // the normal first-run case, not a cache invalidation.
                    if !reason.starts_with("no prior cache entry") {
                        emitter.emit(golem_events::EventKind::InstallCacheMiss {
                            app_name: entry.app_name.clone(),
                            bundle_id: entry.bundle_id.clone(),
                            target: target.clone(),
                            reason,
                        });
                    }
                }
            }
        }

        // Cache miss (or --rebuild): run the install script through the
        // build-once coordinator, then record the new persistent entry.
        let install_result = run_install_with_build_coord(
            &entry.script_path,
            project_root,
            &platform_str,
            device,
            &entry.bundle_id,
            &entry.app_name,
            entry.timeout_ms,
            install_cache,
            Some(&emitter),
        )
        .await;

        if install_result.is_ok() {
            // Capture device-side install state immediately after the
            // install — `device_install_time` lets the next run detect
            // an external reinstall that bumps the device's mtime.
            let info = golem_runner::installed_state::query(device, &entry.bundle_id).await;
            let persisted = golem_runner::installer::PersistedInstall {
                fingerprint: fingerprint.clone(),
                device_install_time: info.install_time,
                installed_version: info.version,
                installed_at: chrono::Utc::now(),
            };
            // Only write when the fingerprint is meaningful — a `None`
            // fingerprint matches no future read, so storing it is wasted
            // disk + risks confusion if the fingerprint becomes
            // computable later.
            if fingerprint.is_some() {
                if let Err(e) = install_cache
                    .set_persistent(&device.udid, &entry.bundle_id, persisted)
                    .await
                {
                    eprintln!("  [install] failed to write cache: {e}");
                }
            }
        }
    }
}

/// Outcome of evaluating the integrity gates for a `(device, bundle)`
/// pair. Hits are summarised optimistically; misses carry a specific
/// reason so a verbose log can explain *why* a build was needed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CacheVerdict {
    /// All gates passed. `label` is the source-fingerprint identity.
    Hit { label: String },
    /// At least one gate failed. `reason` is human-readable.
    Miss { reason: String },
}

/// Pure gate decision over already-fetched inputs. Returns the verdict
/// plus a specific miss reason when applicable. Split out from
/// [`evaluate_cache_gates`] so unit tests can exercise every gate
/// combination without faking I/O.
fn gate_decision(
    entry: Option<&golem_runner::installer::PersistedInstall>,
    current_fingerprint: &golem_runner::fingerprint::Fingerprint,
    info: &golem_runner::installed_state::DeviceInstallInfo,
) -> CacheVerdict {
    if !current_fingerprint.is_some() {
        return CacheVerdict::Miss {
            reason: "fingerprint unavailable (no git, no readable source tree)".into(),
        };
    }
    let Some(entry) = entry else {
        return CacheVerdict::Miss {
            reason: "no prior cache entry for this (device, bundle)".into(),
        };
    };
    if &entry.fingerprint != current_fingerprint {
        return CacheVerdict::Miss {
            reason: format!(
                "source fingerprint changed ({} → {})",
                entry.fingerprint.short_label(),
                current_fingerprint.short_label(),
            ),
        };
    }
    if !info.installed {
        return CacheVerdict::Miss {
            reason: "bundle no longer installed on device".into(),
        };
    }
    // Install-time check: only fires when both sides recorded a
    // timestamp. If either is `None`, the gate is skipped silently —
    // fingerprint+presence alone are sufficient.
    if let (Some(stored), Some(current)) = (entry.device_install_time, info.install_time) {
        // 2s tolerance for filesystem mtime quantisation.
        if (stored - current).num_seconds().abs() > 2 {
            return CacheVerdict::Miss {
                reason: format!(
                    "device install-time differs ({} cached, {} on device — external reinstall?)",
                    stored.format("%Y-%m-%dT%H:%M:%SZ"),
                    current.format("%Y-%m-%dT%H:%M:%SZ"),
                ),
            };
        }
    }
    CacheVerdict::Hit {
        label: current_fingerprint.short_label(),
    }
}

/// Evaluate the three integrity gates and return a [`CacheVerdict`].
///
/// Gates:
/// 1. Persistent entry exists for `(udid, bundle)` with a non-`None`
///    fingerprint
/// 2. Stored fingerprint equals the current source fingerprint
/// 3. Device reports the bundle present AND its current install time
///    matches the stored `device_install_time` (when available) — catches
///    external reinstalls
///
/// Any gate failing → `CacheVerdict::Miss { reason }` with a specific
/// human-readable cause. The caller emits the reason on a verbose log
/// line and falls through to the build path.
async fn evaluate_cache_gates(
    cache: &golem_runner::installer::InstallCache,
    device: &DeviceInfo,
    bundle_id: &str,
    current_fingerprint: &golem_runner::fingerprint::Fingerprint,
) -> CacheVerdict {
    let entry = cache.get_persistent(&device.udid, bundle_id).await;
    let info = golem_runner::installed_state::query(device, bundle_id).await;
    gate_decision(entry.as_ref(), current_fingerprint, &info)
}

/// Run the install script for one `(device, bundle)`, using a
/// per-`(platform, bundle)` build coordinator so only the first device
/// does a full build — subsequent devices pass `install-only` and reuse
/// the built artifact. Updates `install_cache` with the per-device
/// outcome. Returns `Err` when the script fails, a prior build for the
/// same `(platform, bundle)` already failed, or when a prior per-device
/// install is cached as failed.
#[allow(clippy::too_many_arguments)]
async fn run_install_with_build_coord(
    script_path: &Path,
    project_root: &Path,
    platform_str: &str,
    device: &DeviceInfo,
    bundle_id: &str,
    app_name: &str,
    timeout_ms: u64,
    install_cache: &golem_runner::installer::InstallCache,
    emitter: Option<&golem_events::emitter::DeviceEmitter>,
) -> anyhow::Result<()> {
    use golem_runner::installer::{BuildOutcome, BuildRole, InstallOutcome};

    let target = format_install_target(device, platform_str);
    let key = (device.udid.clone(), bundle_id.to_string());

    // Fast-path: per-device outcome already resolved.
    if let Some(outcome) = install_cache.get(&key).await {
        return match outcome {
            InstallOutcome::Succeeded => Ok(()),
            InstallOutcome::FailedScript(e) => Err(anyhow::anyhow!(e)),
            InstallOutcome::FailedNoScript => {
                Err(anyhow::anyhow!("no install_script configured"))
            }
        };
    }

    // Decide role. Builder runs the full script (no `install-only`) and
    // records the outcome; waiters re-install from the already-built
    // artifact. A prior build failure short-circuits without re-running.
    let (install_only, slot) = match install_cache.acquire_build(platform_str, bundle_id).await {
        BuildRole::Build(slot) => (false, Some(slot)),
        BuildRole::Installed(BuildOutcome::Succeeded) => (true, None),
        BuildRole::Installed(BuildOutcome::Failed(err)) => {
            install_cache
                .set(key, InstallOutcome::FailedScript(err.clone()))
                .await;
            return Err(anyhow::anyhow!(
                "build previously failed for {platform_str}/{bundle_id}: {err}"
            ));
        }
    };

    // Builder holds project_lock across the full build so concurrent
    // (platform, bundle) pairs sharing a build tree (monorepo `src-tauri/`)
    // don't corrupt each other. Install-only skips the lock — it doesn't
    // touch the build tree.
    let _proj_guard = match &slot {
        Some(_) => Some(
            install_cache
                .project_lock(project_root, script_path)
                .await
                .lock_owned()
                .await,
        ),
        None => None,
    };

    let mut result = golem_runner::installer::run_install_script(
        script_path,
        project_root,
        platform_str,
        &device.udid,
        bundle_id,
        app_name,
        timeout_ms,
        &target,
        device.os_major,
        install_only,
        emitter,
    )
    .await;

    // Transient-error retry: CoreSimulator's IPC pipe occasionally crashes
    // mid-install on freshly-booted iOS sims (`Mach error -308 (ipc/mig)
    // server died`). The artifact is already built; we just need to retry
    // the install step. Pass `install_only=true` so the script skips its
    // build phase. One retry only — if it fails twice, the issue is
    // probably real, not transient.
    if let Err(ref e) = result {
        if is_transient_install_error(&e.to_string()) {
            if let Some(em) = emitter {
                em.emit(golem_events::EventKind::InstallOutput {
                    app_name: app_name.to_string(),
                    line: "transient install error detected — sleeping 3s and retrying with install-only".into(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let retry = golem_runner::installer::run_install_script(
                script_path,
                project_root,
                platform_str,
                &device.udid,
                bundle_id,
                app_name,
                timeout_ms,
                &target,
                device.os_major,
                true, // install_only — reuse the already-built artifact
                emitter,
            )
            .await;
            if retry.is_ok() {
                result = retry;
            }
        }
    }

    let err_str: Option<String> = result.as_ref().err().map(|e| format!("{e}"));

    if let Some(slot) = slot {
        match &err_str {
            None => slot.record_success().await,
            Some(err) => slot.record_failure(err.clone()).await,
        }
    }

    install_cache
        .set(
            key,
            match err_str {
                None => InstallOutcome::Succeeded,
                Some(err) => InstallOutcome::FailedScript(err),
            },
        )
        .await;

    result
}


/// Build a `SuitePlanned` event payload from the parsed suite. Pre-formats
/// the per-run lines, install entries, and device availability as
/// human-readable strings so the stream renderer can print them verbatim
/// and the orchestrator forwarder can relay the same payload to clients.
fn build_suite_planned_event(parsed: &golem_orchestrator::ParsedSuite) -> golem_events::EventKind {
    let flow_runs: Vec<String> = parsed
        .flow_runs
        .iter()
        .enumerate()
        .map(|(i, run)| {
            let flow_name = parsed
                .flows
                .get(run.flow_idx)
                .map(|f| f.flow.flow.name.as_str())
                .unwrap_or("?");
            let slots: Vec<String> = run.slots.iter().map(golem_orchestrator::describe_slot).collect();
            format!("#{} {}: {}", i + 1, flow_name, slots.join(" + "))
        })
        .collect();

    let install_entries: Vec<String> = parsed
        .install_matrix
        .iter()
        .map(|e| format!("{} {} → {}", e.platform, e.app_name, e.bundle_id))
        .collect();

    golem_events::EventKind::SuitePlanned {
        flow_runs,
        install_entries,
        device_availability: parsed.device_availability.clone(),
    }
}

/// Check whether an `InstallEntry`'s per-app `devices` constraints permit the
/// given device. Safety net — the device resolver already filters upstream,
/// but a stray call from a future code path shouldn't install onto a device
/// the app explicitly excludes.
///
/// A device matches the entry if it matches ANY one of the constraints.
/// Within one constraint, every set field (device_type, physical, name,
/// playstore) must match — unset fields are wildcards. `os` is not rechecked
/// here (the resolver owns version matching via `device_matches_slot`);
/// `accessibility_label` is a UI-element field with no `DeviceInfo` counterpart.
fn device_matches_entry_constraints(device: &DeviceInfo, entry: &InstallEntry) -> bool {
    if entry.device_constraints.is_empty() {
        return true;
    }
    entry.device_constraints.iter().any(|c| {
        // device_type: if set, device's type must appear in the requested list.
        if let Some(sv) = &c.device_type {
            let matches_type = sv.to_vec().iter().any(|t| {
                matches!(
                    (t.as_str(), device.device_type),
                    ("phone", golem_devices::DeviceType::Phone)
                        | ("tablet", golem_devices::DeviceType::Tablet)
                )
            });
            if !matches_type {
                return false;
            }
        }
        // `hardware` values: "virtual" (sim/emulator) or "real" (physical).
        // Array form = match any listed kind.
        if let Some(sv) = &c.hardware {
            let matches_hw = sv.to_vec().iter().any(|h| {
                matches!(
                    (h.as_str(), device.physical),
                    ("virtual", false) | ("real", true)
                )
            });
            if !matches_hw {
                return false;
            }
        } else if device.physical {
            // Default (no `hardware` key) = virtual-only.
            return false;
        }
        if let Some(name) = &c.name {
            if &device.name != name {
                return false;
            }
        }
        if let Some(ps) = c.playstore {
            if device.playstore != ps {
                return false;
            }
        }
        true
    })
}

// Public wrappers (used by golem tree)
pub fn find_companion_path_public(platform: Platform) -> Result<String> {
    find_companion_path(platform)
}
pub fn find_android_apk_public() -> Result<String> {
    find_android_apk()
}
pub fn find_android_main_apk_public() -> Option<String> {
    find_android_main_apk()
}

/// Discover booted devices and start companions for all platforms.
/// Used by `golem tree` and potentially other commands that need companions.
pub async fn start_companions_public(
    platform_filter: Option<&str>,
) -> Result<Vec<(u16, golem_driver::CompanionHealth)>> {
    let mut platforms = Vec::new();
    if platform_filter.is_none() || platform_filter == Some("ios") {
        platforms.push(Platform::Ios);
    }
    if platform_filter.is_none() || platform_filter == Some("android") {
        platforms.push(Platform::Android);
    }

    let (reg_state, _rx) = crate::registration::RegistrationState::new();
    let reg_port = crate::registration::start_registration_server(reg_state.clone()).await?;

    let mut results = Vec::new();

    for platform in platforms {
        let devices = match platform {
            Platform::Ios => golem_devices::ios::discover_ios_devices().await.unwrap_or_default(),
            Platform::Android => golem_devices::android::discover_android_devices().await.unwrap_or_default(),
        };

        let booted: Vec<_> = devices.into_iter()
            .filter(|d| d.state == golem_devices::DeviceState::Booted)
            .collect();

        if booted.is_empty() {
            continue;
        }

        let device = &booted[0];
        let companion_path = match find_companion_path(platform) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if platform == Platform::Android {
            if let Ok(apk) = find_android_apk() {
                let cmd = golem_devices::lifecycle::install_companion_command(device, &apk);
                let _ = golem_devices::lifecycle::run_command_public(&cmd, "install test APK").await;
            }
            if let Some(main) = find_android_main_apk() {
                let cmd = golem_devices::lifecycle::install_companion_command(device, &main);
                let _ = golem_devices::lifecycle::run_command_public(&cmd, "install main APK").await;
            }
        } else {
            let _ = golem_devices::lifecycle::build_companion(device, &companion_path).await;
        }

        if let Ok(()) = golem_devices::lifecycle::spawn_companion_with_reg(
            device, &companion_path, 0, Some(reg_port),
        ).await {
            let mut rx = reg_state.subscribe();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        if let Ok(id) = msg {
                            if let Some(comp) = reg_state.get(&id) {
                                if platform == Platform::Android {
                                    let fwd = golem_devices::lifecycle::port_forward_command(device, comp.port);
                                    let _ = golem_devices::lifecycle::run_command_public(&fwd, "port forward").await;
                                }
                                let client = golem_driver::common::CompanionClient::new(comp.port);
                                if let Ok(health) = client.wait_for_health(std::time::Duration::from_secs(15)).await {
                                    results.push((comp.port, health));
                                }
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        eprintln!("  [companion] startup timed out for {platform}");
                        break;
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Public wrapper for scan_companions (used by `golem tree`).
pub async fn scan_companions_public() -> Vec<(u16, golem_driver::CompanionHealth)> {
    scan_companions().await
}

/// Scan ports for running companion servers.
///
/// Checks ports in the companion range concurrently for a responding
/// /health endpoint. Returns a list of (port, health) for all found.
/// Fast — unused ports return "connection refused" instantly.
async fn scan_companions() -> Vec<(u16, golem_driver::CompanionHealth)> {
    use golem_devices::resource_manager::{PORT_RANGE_START, PORT_RANGE_END};
    use golem_driver::common::CompanionClient;

    let mut handles = Vec::new();
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        handles.push(tokio::spawn(async move {
            let client = CompanionClient::new(port);
            match client.check_health().await {
                Ok(health) => Some((port, health)),
                Err(_) => None,
            }
        }));
    }

    let mut found = Vec::new();
    for handle in handles {
        if let Ok(Some(result)) = handle.await {
            found.push(result);
        }
    }
    found
}

/// Find a port for a device: reuse an existing matching companion or allocate a free port.
async fn find_or_allocate_port(device: &DeviceInfo, platform: Platform) -> Result<u16> {
    let golem_version = env!("CARGO_PKG_VERSION");
    let platform_str = match platform {
        Platform::Ios => "ios",
        Platform::Android => "android",
    };

    let companions = scan_companions().await;

    // Try to find an existing companion for this device with matching version.
    // First try exact match by device name or ID. Then fall back to matching
    // by platform+version if there's only one companion for that platform
    // (handles Android where the companion can't report the ADB serial).
    let platform_companions: Vec<_> = companions
        .iter()
        .filter(|(_, h)| h.platform == platform_str && h.version == golem_version)
        .collect();

    for (port, health) in &platform_companions {
        if health.device_id == device.udid
            || health.device_name == device.name
            || health.device_name == device.udid
        {
            return Ok(*port);
        }
    }

    // Android-only fallback: the Android companion can't report its own
    // ADB serial via UIDevice (the equivalent doesn't exist), so when
    // there's exactly one matching companion we assume it's ours. iOS
    // *can* report device id/name reliably, so don't fall through there
    // — otherwise a stale iPhone companion from a previous run will be
    // reused when the slot is targeting iPad, and we never spin up a
    // companion on the right simulator.
    if platform == Platform::Android && platform_companions.len() == 1 {
        return Ok(platform_companions[0].0);
    }

    // No match — find first free port
    let used_ports: Vec<u16> = companions.iter().map(|(p, _)| *p).collect();
    use golem_devices::resource_manager::{PORT_RANGE_START, PORT_RANGE_END};
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        if !used_ports.contains(&port) {
            return Ok(port);
        }
    }

    anyhow::bail!("No free companion ports in range {PORT_RANGE_START}-{PORT_RANGE_END}")
}

/// Launch a companion using the registration server.
/// Returns the port the companion registered on.
async fn ensure_companion_with_reg(
    device: &DeviceInfo,
    platform: Platform,
    reg_port: u16,
    reg_state: &crate::registration::RegistrationState,
    event_tx: &golem_events::channel::EventSender,
) -> Result<u16> {
    event_tx.emit(
        golem_events::DeviceId(format!("{platform}/{}", device.name)),
        golem_events::EventKind::CompanionStarting {
            platform: platform.to_string(),
            device_name: device.name.clone(),
        },
    );
    let companion_path = find_companion_path(platform)?;

    // Install/build companion
    if platform == Platform::Android {
        let apk_path = find_android_apk()?;
        let main_apk_path = find_android_main_apk();
        // Install APKs only (no port forward — that happens after registration)
        let install_main = golem_devices::lifecycle::install_companion_command(device, &apk_path);
        let _ = golem_devices::lifecycle::run_command_public(&install_main, "install test APK").await;
        if let Some(ref main_path) = main_apk_path {
            let install_app = golem_devices::lifecycle::install_companion_command(device, main_path);
            let _ = golem_devices::lifecycle::run_command_public(&install_app, "install main APK").await;
        }
    } else {
        golem_devices::lifecycle::build_companion(device, &companion_path).await?;
    }

    // Launch companion with registration port
    golem_devices::lifecycle::spawn_companion_with_reg(
        device, &companion_path, 0, Some(reg_port),
    ).await?;

    // Wait for the companion we just spawned to register (up to 60s).
    // We require two things to match before returning a port:
    //   1. The registered device_id matches `device.udid` (filters out
    //      other simulators' companions registering at the same time).
    //   2. A live `/health` on the assigned port also reports the
    //      expected device. This is the critical check: registration
    //      assigns a port based on a free-bind probe, but a stale
    //      companion from a previous golem run can already be bound
    //      to that port — the new companion then re-registers on a
    //      different port, and any code that trusted only the first
    //      registration would route to the wrong simulator.
    // (iOS and Android both report a canonical device id; Android via
    // the explicit `device_serial` env var, iOS via `UIDevice.current
    // .identifierForVendor`.)
    let mut rx = reg_state.subscribe();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        tokio::select! {
            msg = rx.recv() => {
                if let Ok(registered_id) = msg {
                    if let Some(comp) = reg_state.get(&registered_id) {
                        // Strict UDID match. `device_name` is shown to
                        // users but isn't unique (two iPhones can share
                        // a name) — UDID is canonical on both platforms.
                        if comp.device_id != device.udid {
                            continue;
                        }
                        // For Android, set up ADB forward for the assigned port
                        if platform == Platform::Android {
                            let fwd = golem_devices::lifecycle::port_forward_command(device, comp.port);
                            let _ = golem_devices::lifecycle::run_command_public(&fwd, "port forward").await;
                        }
                        // Wait briefly for the companion to start serving after registration
                        let client = golem_driver::common::CompanionClient::new(comp.port);
                        let health = match client
                            .wait_for_health(std::time::Duration::from_secs(15))
                            .await
                        {
                            Ok(h) => h,
                            Err(_) => continue, // companion never came up on this port; keep waiting
                        };
                        // Final cross-check: the live process on this
                        // port must report the same UDID we asked for.
                        // Without this, a stale companion bound to the
                        // assigned port would be silently routed to.
                        if health.device_id != device.udid {
                            continue;
                        }
                        return Ok(comp.port);
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                anyhow::bail!("Companion did not register within 60 seconds");
            }
        }
    }
}


/// Find the companion project path for the given platform.
fn find_companion_path(platform: Platform) -> Result<String> {
    // Check extracted embedded companions first
    if let Ok(paths) = crate::companions::ensure_extracted() {
        match platform {
            Platform::Ios => {
                if let Some(ref ios_dir) = paths.ios_products {
                    // For iOS, return the directory containing the .xctestrun file
                    return Ok(ios_dir.to_string_lossy().into_owned());
                }
            }
            Platform::Android => {
                if let Some(ref apk) = paths.android_apk {
                    if let Some(parent) = apk.parent() {
                        return Ok(parent.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    let relative = match platform {
        Platform::Ios => "companions/ios/GolemRunner.xcodeproj",
        Platform::Android => "companions/android",
    };

    // Check relative to current working directory
    if std::path::Path::new(relative).exists() {
        return Ok(relative.to_string());
    }

    // Check relative to golem binary location
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join(relative);
            if path.exists() {
                return Ok(path.to_string_lossy().into_owned());
            }
        }
    }

    anyhow::bail!(
        "Companion not found. Embedded companions may not have been built."
    )
}

/// Find the Android companion test APK.
fn find_android_apk() -> Result<String> {
    if let Ok(paths) = crate::companions::ensure_extracted() {
        if let Some(ref apk) = paths.android_apk {
            if apk.exists() {
                return Ok(apk.to_string_lossy().into_owned());
            }
        }
    }

    let relative = "companions/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk";
    if std::path::Path::new(relative).exists() {
        return Ok(relative.to_string());
    }
    anyhow::bail!("Android companion test APK not found.")
}

/// Find the Android companion main APK (optional, needed for fresh installs).
fn find_android_main_apk() -> Option<String> {
    if let Ok(paths) = crate::companions::ensure_extracted() {
        if let Some(ref apk) = paths.android_main_apk {
            if apk.exists() {
                return Some(apk.to_string_lossy().into_owned());
            }
        }
    }

    let relative = "companions/android/app/build/outputs/apk/debug/app-debug.apk";
    if std::path::Path::new(relative).exists() {
        return Some(relative.to_string());
    }
    None
}

/// Execute a flow on a single device. This is a free function (not a method)
/// so it can be used with `tokio::spawn` which requires `'static` futures.
/// All parameters are owned values.
#[allow(clippy::too_many_arguments)]
async fn run_flow_on_device(
    flow: FlowFile,
    flow_name: String,
    flow_dir: PathBuf,
    device: DeviceInfo,
    platform: Platform,
    port: u16,
    seed: Option<u64>,
    start_block: Option<String>,
    cli_vars: Vec<(String, String)>,
    output_dir: PathBuf,
    no_results: bool,
    install_cache: golem_runner::installer::InstallCache,
    project_root: PathBuf,
    barrier: golem_runner::barrier::FailureBarrier,
    no_perf: bool,
    event_sender: Option<golem_events::channel::EventSender>,
) -> FlowReport {
    let start = Instant::now();
    let device_name = device.name.clone();
    let device_label = format!("{platform}/{device_name}");

    let bundle_id = flow
        .flow
        .apps
        .first()
        .and_then(|a| a.bundle.clone())
        .unwrap_or_else(|| "fail.golem.test".to_string());

    let driver: Box<dyn PlatformDriver> = match platform {
        Platform::Ios => Box::new(IosDriver::new(device.udid.clone(), bundle_id.clone(), port)),
        Platform::Android => Box::new(AndroidDriver::new(device.udid.clone(), bundle_id.clone(), port)),
    };

    // Resolve perf setting: CLI --no-perf overrides flow perf option (default: true)
    let flow_perf = flow.flow.options.as_ref().and_then(|o| o.perf).unwrap_or(true);
    let perf_enabled = !no_perf && flow_perf;

    let companion_port = if platform == Platform::Android { Some(port) } else { None };
    let apps: Vec<(String, String)> = flow
        .flow
        .apps
        .iter()
        .filter_map(|a| a.bundle.clone().map(|b| (a.name.clone(), b)))
        .collect();
    let collector = if perf_enabled {
        Some(golem_runner::perf::PerfCollectorSet::new(
            &apps,
            platform,
            device.udid.clone(),
            companion_port,
        ))
    } else {
        None
    };

    let mut vars = VariableStore::new();

    // Inject flow-level variables ([flow.vars]).
    if !flow.flow.vars.is_empty() {
        let mut flow_scope = golem_vars::Scope::new(golem_vars::ScopeLevel::Flow);
        for (k, v) in &flow.flow.vars {
            flow_scope.set(k.clone(), golem_vars::VarValue::String(v.clone()));
        }
        vars.push_scope(flow_scope);
    }

    // Inject CLI --var overrides (higher priority than flow vars).
    if !cli_vars.is_empty() {
        let mut cli_scope = golem_vars::Scope::new(golem_vars::ScopeLevel::Cli);
        for (k, v) in &cli_vars {
            cli_scope.set(k.clone(), golem_vars::VarValue::String(v.clone()));
        }
        vars.push_scope(cli_scope);
    }

    // Create seeded RNG: --seed for deterministic, random otherwise.
    // Always capture the actual seed for reproducibility in reports.
    use rand::SeedableRng;
    let actual_seed: u64 = seed.unwrap_or_else(rand::random);
    let rng = rand_chacha::ChaCha8Rng::seed_from_u64(actual_seed);

    let capture_config = {
        let mut cfg = CaptureConfig {
            output_dir,
            flow_name: flow_name.clone(),
            device_name: device_name.clone(),
            write_to_disk: !no_results,
            ..CaptureConfig::default()
        };
        if let Some(ref opts) = flow.flow.options {
            if let Some(v) = opts.screenshot_on_failure {
                cfg.screenshot_on_failure = v;
            }
        }
        cfg
    };
    let device_emitter = event_sender.map(|sender| {
        golem_events::emitter::DeviceEmitter::new(
            sender,
            golem_events::DeviceId(device_label.clone()),
        )
    });
    let mut ctx = ExecutionContext {
        flow_dir: &flow_dir,
        project_root: &project_root,
        capture_config: &capture_config,
        flow_name: &flow_name,
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: Some(&device),
        perf_collector: collector.as_ref(),
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: device_emitter.as_ref(),
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rng),
    };

    // Run install scripts for all apps in this flow on this device (unless
    // already done or previously failed). If install fails or a prior install
    // for this (device, bundle) failed, skip the flow on this device.
    for app in &flow.flow.apps {
        let bundle_for_install = match app.bundle.as_ref() {
            Some(b) => b.clone(),
            None => {
                let reason = format!(
                    "app '{}' has no bundle id — add one to [[flow.apps]] or [[apps]] in golem.toml",
                    app.name
                );
                ctx.emit(golem_events::EventKind::FlowSkipped {
                    flow_name: flow_name.clone(),
                    reason: reason.clone(),
                });
                return FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: Some(actual_seed),
                    screenshot_path: None,
                    device_name: Some(device_label.clone()),
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
                    covered_axes: Vec::new(),
                    started_at: None,
                    finished_at: None,
                };
            }
        };
        let key = (device.udid.clone(), bundle_for_install.clone());
        match install_cache.get(&key).await {
            Some(golem_runner::installer::InstallOutcome::Succeeded) => continue,
            Some(golem_runner::installer::InstallOutcome::FailedScript(err)) => {
                let reason = format!(
                    "install_script failed earlier for {} on {device_name}: {err}",
                    bundle_for_install
                );
                ctx.emit(golem_events::EventKind::FlowSkipped {
                    flow_name: flow_name.clone(),
                    reason: reason.clone(),
                });
                return FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: Some(actual_seed),
                    screenshot_path: None,
                    device_name: Some(device_label.clone()),
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
                    covered_axes: Vec::new(),
                    started_at: None,
                    finished_at: None,
                };
            }
            Some(golem_runner::installer::InstallOutcome::FailedNoScript) => {
                let reason = format!(
                    "{} not installed on {device_name} and no install_script configured. \
                     Add install_script to [[flow.apps]] or [install] in golem.toml.",
                    bundle_for_install
                );
                ctx.emit(golem_events::EventKind::FlowSkipped {
                    flow_name: flow_name.clone(),
                    reason: reason.clone(),
                });
                return FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: Some(actual_seed),
                    screenshot_path: None,
                    device_name: Some(device_label.clone()),
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
                    covered_axes: Vec::new(),
                    started_at: None,
                    finished_at: None,
                };
            }
            None => {}
        }

        // Not cached yet — resolve the script for this platform.
        let platform_str = match platform {
            Platform::Ios => "ios",
            Platform::Android => "android",
        };
        let script_rel = app.install_script.as_ref()
            .and_then(|v| v.for_platform(platform_str))
            .map(|s| s.to_string());
        let timeout_ms = app.install_timeout_ms
            .unwrap_or(golem_runner::installer::DEFAULT_INSTALL_TIMEOUT_MS);

        if let Some(rel) = script_rel {
            let script_path = project_root.join(&rel);
            let result = run_install_with_build_coord(
                &script_path,
                &project_root,
                platform_str,
                &device,
                &bundle_for_install,
                &app.name,
                timeout_ms,
                &install_cache,
                device_emitter.as_ref(),
            )
            .await;
            if let Err(e) = result {
                let err_str = format!("{e}");
                let reason = format!(
                    "install_script failed for {} on {device_name}: {err_str}",
                    bundle_for_install
                );
                ctx.emit(golem_events::EventKind::FlowSkipped {
                    flow_name: flow_name.clone(),
                    reason: reason.clone(),
                });
                return FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: Vec::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: Some(actual_seed),
                    screenshot_path: None,
                    device_name: Some(device_label.clone()),
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
                    covered_axes: Vec::new(),
                    started_at: None,
                    finished_at: None,
                };
            }
        }
    }

    ctx.emit(golem_events::EventKind::FlowStarted {
        flow_name: flow_name.clone(),
        os_major: device.os_major,
    });
    // CLI --start takes precedence over flow-level start field.
    let effective_start = start_block.as_deref().or(flow.flow.start.as_deref());
    let base_timeout = flow.flow.options.as_ref()
        .and_then(|o| o.step_timeout)
        .unwrap_or(golem_runner::policy::DEFAULT_BASE_TIMEOUT_MS);
    match execute_flow(&flow, driver.as_ref(), &mut vars, effective_start, base_timeout, &mut ctx, Some(&barrier)).await {
        Ok(result) => {
            if !result.success {
                if result.barrier_aborted {
                    eprintln!("  [{device_label}] Aborted: another device failed at this point");
                } else {
                    if let Some(ref block) = result.failed_block {
                        let step_info = match (result.failed_step, &result.failed_action) {
                            (Some(s), Some(a)) => format!(" step {s} ({a})"),
                            (Some(s), None) => format!(" step {s}"),
                            _ => String::new(),
                        };
                        eprintln!("  [{device_label}] Failed in {block}{step_info}");
                    }
                    if let Some(ref reason) = result.failed_reason {
                        eprintln!("  [{device_label}] Error: {reason}");
                    }
                }
            }
            for w in &result.warnings {
                eprintln!("  [{device_label}] Warning: {w}");
            }
            let duration_ms = start.elapsed().as_millis() as u64;
            ctx.emit(golem_events::EventKind::FlowFinished {
                flow_name: flow_name.clone(),
                success: result.success,
                duration_ms,
                seed: actual_seed,
                os_major: device.os_major,
            });
            FlowReport {
                flow_name,
                success: result.success,
                step_results: Vec::new(),
                warnings: result.warnings,
                duration_ms,
                seed: Some(actual_seed),
                screenshot_path: None,
                device_name: Some(device_label),
                os_major: None,
                perf_snapshots: result.perf_snapshots,
                skipped_reason: None,
                covered_axes: device_covered_axes(&device),
                started_at: None,
                finished_at: None,
            }
        }
        Err(e) => {
            eprintln!("  [{device_label}] Error: {e:#}");
            let duration_ms = start.elapsed().as_millis() as u64;
            ctx.emit(golem_events::EventKind::FlowFinished {
                flow_name: flow_name.clone(),
                success: false,
                duration_ms,
                seed: actual_seed,
                os_major: device.os_major,
            });
            FlowReport {
                flow_name,
                success: false,
                step_results: Vec::new(),
                warnings: vec![format!("Execution error: {e}")],
                duration_ms,
                seed: Some(actual_seed),
                screenshot_path: None,
                device_name: Some(device_label),
                os_major: None,
                perf_snapshots: vec![],
                skipped_reason: None,
                covered_axes: device_covered_axes(&device),
                started_at: None,
                finished_at: None,
            }
        }
    }
}

/// Discover ALL devices for the given platform (booted and shutdown).
async fn discover_all_devices(platform: Option<Platform>) -> Result<Vec<DeviceInfo>> {
    let want_ios = matches!(platform, None | Some(Platform::Ios));
    let want_android = matches!(platform, None | Some(Platform::Android));

    let mut out = Vec::new();

    if want_ios {
        // Soft-fail on iOS when a slot is platform-agnostic — a host with
        // only Android tooling shouldn't block discovery on the iOS side.
        match (golem_devices::ios::discover_ios_devices().await, platform) {
            (Ok(ios), _) => out.extend(ios),
            (Err(e), Some(Platform::Ios)) => return Err(e),
            (Err(_), _) => {}
        }
    }

    if want_android {
        // `discover_android_devices` enumerates AVDs from
        // `~/.android/avd` (so shutdown emulators are visible for
        // auto-boot) and merges in any running devices not backed by an
        // AVD (physical devices, unparseable configs). Symmetric with
        // iOS where `discover_ios_devices` returns booted + shutdown.
        match golem_devices::android::discover_android_devices().await {
            Ok(android) => out.extend(android),
            Err(e) if platform == Some(Platform::Android) => return Err(e),
            Err(_) => {}
        }
    }

    Ok(out)
}

/// Filter `booted` to those currently unallocated (per `ResourceManager`),
/// then rank the survivors by install-cache hits and return the best.
/// Returns `None` when every booted candidate is busy — caller decides
/// whether to wait, auto-boot a shutdown device, or fail.
#[allow(clippy::too_many_arguments)]
async fn try_pick_free(
    booted: &[&DeviceInfo],
    platform: Option<Platform>,
    slot: Option<&DeviceSlot>,
    resource_mgr: &golem_devices::resource_manager::ResourceManager,
    install_cache: Option<&golem_runner::installer::InstallCache>,
    install_matrix: &[InstallEntry],
) -> Option<DeviceInfo> {
    let free: Vec<&DeviceInfo> = booted
        .iter()
        .copied()
        .filter(|d| resource_mgr.port_for(&d.udid).is_none())
        .collect();
    if free.is_empty() {
        return None;
    }
    let pick = rank_by_install_cache(&free, platform, slot, install_cache, install_matrix).await;
    Some(pick.clone())
}

/// Pick the free candidate with the most install-cache `Succeeded` hits
/// for the slot's apps — saves re-running install scripts on a cold device
/// when a warm one is free. Ties are broken by the input order (stable).
///
/// If `install_cache` is `None`, `slot` is `None`, or the install matrix has
/// no entries matching this (platform, app), every candidate scores 0 and the
/// first is returned. That preserves the pre-ranking behaviour for test
/// harnesses and platform-only calls.
///
/// `Failed*` cache entries don't count — the suite still needs a device we
/// haven't yet installed on, and the per-flow skip logic handles re-use of
/// known-failed devices upstream.
async fn rank_by_install_cache<'a>(
    free: &'a [&'a DeviceInfo],
    platform: Option<Platform>,
    slot: Option<&DeviceSlot>,
    install_cache: Option<&golem_runner::installer::InstallCache>,
    install_matrix: &[InstallEntry],
) -> &'a DeviceInfo {
    let (Some(cache), Some(s)) = (install_cache, slot) else {
        return free[0];
    };
    let mut best: &DeviceInfo = free[0];
    let mut best_score = 0usize;
    for (i, dev) in free.iter().enumerate() {
        // Bundle list is per-device when the slot is platform-agnostic
        // (mixed-platform free pool). Each device picks its own platform's
        // install_matrix entries. When the slot pins a platform, we still
        // honour it as a sanity filter.
        let dev_platform = platform.unwrap_or(dev.platform);
        let bundles: Vec<&str> = s
            .apps
            .iter()
            .filter_map(|app_name| {
                install_matrix
                    .iter()
                    .find(|e| e.platform == dev_platform && &e.app_name == app_name)
                    .map(|e| e.bundle_id.as_str())
            })
            .collect();
        if bundles.is_empty() {
            continue;
        }
        let mut score = 0usize;
        for b in &bundles {
            if let Some(golem_runner::installer::InstallOutcome::Succeeded) = cache
                .get(&(dev.udid.clone(), (*b).to_string()))
                .await
            {
                score += 1;
            }
        }
        if i == 0 || score > best_score {
            best = dev;
            best_score = score;
        }
    }
    best
}

/// Find the best available device for a platform, honouring slot
/// requirements if provided (os_version, device_type, physical, name).
///
/// Priority:
/// 1. Free booted device matching slot → return one, preferring candidates
///    with the most install-cache hits for this slot's apps (ranking only
///    within the free set, so busy devices never block parallelism)
/// 2. No matching booted → auto-boot the best shutdown device that
///    matches the slot (highest `os_major` tie-break among equally-matching
///    candidates, which naturally honours `Exact(N)` after filter)
/// 3. No compatible devices at all → auto-create if `create_if_missing`
/// 4. All matching booted busy → wait up to 20 minutes
/// 5. No compatible devices and `create_if_missing` false → fail
///
/// When `slot` is `None` (direct test harness, no plan phase) we fall
/// back to the pre-slot behaviour: match by platform only.
#[allow(clippy::too_many_arguments)]
async fn find_available_device(
    platform: Option<Platform>,
    slot: Option<&DeviceSlot>,
    resource_mgr: &golem_devices::resource_manager::ResourceManager,
    create_if_missing: bool,
    event_tx: &golem_events::channel::EventSender,
    install_cache: Option<&golem_runner::installer::InstallCache>,
    install_matrix: &[InstallEntry],
) -> Result<DeviceInfo> {
    let all_devices = discover_all_devices(platform).await?;

    let compatible: Vec<&DeviceInfo> = all_devices
        .iter()
        .filter(|d| platform.map(|p| d.platform == p).unwrap_or(true))
        .filter(|d| match slot {
            Some(s) => device_matches_slot(d, s),
            None => true,
        })
        .collect();

    // Separate booted from shutdown
    let booted: Vec<&DeviceInfo> = compatible
        .iter()
        .filter(|d| d.state == DeviceState::Booted)
        .copied()
        .collect();

    let shutdown: Vec<&DeviceInfo> = compatible
        .iter()
        .filter(|d| d.state == DeviceState::Shutdown)
        .copied()
        .collect();

    // Step 1: Try to find a free booted device, preferring one whose install
    // cache already has `Succeeded` entries for the slot's apps — saves
    // re-running the install script. Ranking runs over *free* candidates only,
    // so a busy-but-hot device can't serialise a cold-free device (parallel
    // FlowRuns always grab a free one, picking cold over waiting).
    if !booted.is_empty() {
        // Booted count already reported via SuitePlanned/device_availability.
        if let Some(pick) =
            try_pick_free(&booted, platform, slot, resource_mgr, install_cache, install_matrix)
                .await
        {
            return Ok(pick);
        }
        // All booted are busy — fall through to wait loop below
    }

    // Step 2: No matching booted devices — auto-boot the best matching shutdown.
    // `compatible` is already slot-filtered, so `max_by_key(os_major)` picks
    // the highest version among matches; for `Exact(N)` every match has the
    // same major so the tie-break is a no-op.
    if booted.is_empty() && !shutdown.is_empty() {
        let best = shutdown.iter()
            .max_by_key(|d| d.os_major)
            .expect("shutdown non-empty");
        let shape = slot
            .map(golem_orchestrator::shape_label)
            .unwrap_or_else(|| best.platform.to_string());
        event_tx.emit(
            golem_events::DeviceId("suite".into()),
            golem_events::EventKind::DeviceAutoBoot {
                device_name: best.name.clone(),
                slot_shape: shape.clone(),
            },
        );
        let boot_start = Instant::now();
        // Returned DeviceInfo carries the post-boot udid (Android: real
        // emulator-NNNN serial, not the AVD identifier). Subsequent
        // adb-based operations need that to address the device.
        let booted_device = golem_devices::lifecycle::boot_device(best).await?;
        event_tx.emit(
            golem_events::DeviceId("suite".into()),
            golem_events::EventKind::DeviceAutoBootFinished {
                device_name: best.name.clone(),
                slot_shape: shape,
                duration_ms: boot_start.elapsed().as_millis() as u64,
            },
        );
        resource_mgr.mark_golem_booted(booted_device.clone());
        return Ok(booted_device);
    }

    // Step 3: No compatible devices at all — auto-create or fail.
    // Honour the slot's `device_type` (phone vs tablet) and `os_version`
    // (specific iOS major / Android API) so a flow needing an iPad on
    // iOS 18 doesn't get a phone on latest.
    if compatible.is_empty() {
        if create_if_missing {
            // Guard: constraints auto-create can't satisfy. Bail fast with
            // actionable errors rather than silently creating a sim that
            // the scheduler will reject as non-matching.
            if let Some(s) = slot {
                if s.physical == Some(true) {
                    anyhow::bail!(
                        "slot requires physical device ({}); auto-create cannot \
                         provision real hardware. Connect a matching device or \
                         remove the `physical` constraint.",
                        golem_orchestrator::describe_slot(s)
                    );
                }
                if let Some(n) = &s.name {
                    anyhow::bail!(
                        "slot pins device name `{n}` which isn't connected/booted; \
                         auto-create cannot guess custom device configs. Either \
                         connect the named device or remove the `name` constraint."
                    );
                }
            }
            // Auto-create needs a concrete platform — we can't provision a
            // device without knowing iOS vs Android. A platform-None slot
            // (partial-axis emission with no os pin) reaching here means
            // neither platform has any matching device booted/shutdown.
            let target_platform = match platform.or_else(|| slot.and_then(|s| s.platform)) {
                Some(p) => p,
                None => anyhow::bail!(
                    "cannot auto-create a platform-agnostic slot ({}) — \
                     specify `os = \"ios:...\"` or `os = \"android:...\"` on the \
                     `[[flow.apps.devices]]` block, or boot a matching device \
                     manually.",
                    slot.map(golem_orchestrator::describe_slot)
                        .unwrap_or_else(|| "unconstrained".to_string())
                ),
            };
            let requested_type = slot
                .and_then(|s| s.device_type)
                .unwrap_or(golem_devices::DeviceType::Phone);
            let requested_os = slot.and_then(|s| s.os_version.clone());
            let requested_playstore = slot.and_then(|s| s.playstore);
            eprintln!("  [devices] no {target_platform} device found — creating one...");
            let config = golem_devices::concurrency::ConcurrencyConfig::default();
            let created = golem_devices::lifecycle::auto_create_device(
                target_platform,
                requested_type,
                requested_os,
                requested_playstore,
                &config,
            ).await?;
            resource_mgr.mark_golem_booted(created.clone());
            return Ok(created);
        } else {
            let label = platform
                .map(|p| p.to_string())
                .unwrap_or_else(|| "matching".to_string());
            anyhow::bail!(
                "No {label} devices found. Use create_if_missing = true to auto-create, \
                 or boot a simulator/emulator manually."
            );
        }
    }

    // Step 4: All booted devices are busy — wait for one to free up
    let timeout = std::time::Duration::from_secs(1200); // 20 minutes
    let deadline = tokio::time::Instant::now() + timeout;
    let mut emitted_waiting = false;

    let wait_label = platform
        .map(|p| p.to_string())
        .or_else(|| slot.map(golem_orchestrator::shape_label))
        .unwrap_or_else(|| "any".to_string());

    loop {
        if let Some(pick) =
            try_pick_free(&booted, platform, slot, resource_mgr, install_cache, install_matrix)
                .await
        {
            return Ok(pick);
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "Timed out waiting for a free {wait_label} device (all {} are in use)",
                booted.len()
            );
        }

        if !emitted_waiting {
            event_tx.emit(
                golem_events::DeviceId("suite".into()),
                golem_events::EventKind::ResourcesWaiting {
                    platform: wait_label.clone(),
                },
            );
            emitted_waiting = true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Summary statistics for a completed suite run.
pub struct SuiteStats {
    /// Total number of flows in the suite.
    pub total: usize,
    /// Number of flows that passed.
    pub passed: usize,
    /// Number of flows that failed.
    pub failed: usize,
}

/// Compute aggregate statistics from a [`SuiteReport`].
pub fn suite_stats(report: &SuiteReport) -> SuiteStats {
    SuiteStats {
        total: report.flows.len(),
        passed: report.flows.iter().filter(|f| f.is_passed()).count(),
        failed: report.flows.iter().filter(|f| f.is_failed()).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a passing FlowReport with the given name.
    fn passing_flow(name: &str) -> FlowReport {
        FlowReport {
            flow_name: name.to_string(),
            success: true,
            step_results: Vec::new(),
            warnings: Vec::new(),
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            started_at: None,
            finished_at: None,
        }
    }

    /// Helper to create a failing FlowReport with the given name.
    fn failing_flow(name: &str) -> FlowReport {
        FlowReport {
            flow_name: name.to_string(),
            success: false,
            step_results: Vec::new(),
            warnings: Vec::new(),
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            started_at: None,
            finished_at: None,
        }
    }

    // ---------------------------------------------------------------
    // 1. SuiteConfig defaults are correct
    // ---------------------------------------------------------------
    #[test]
    fn suite_config_defaults() {
        let config = SuiteConfig::default();
        assert!(!config.no_clean);
        assert!(!config.no_teardown);
        assert!(!config.keep_devices);
        assert!(config.seed.is_none());
        assert!(!config.no_perf);
        assert!(!config.rebuild);
        assert!(!config.no_build);
    }

    // ---------------------------------------------------------------
    // gate_decision — every combination
    // ---------------------------------------------------------------
    fn make_entry(
        fp: golem_runner::fingerprint::Fingerprint,
        install_time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> golem_runner::installer::PersistedInstall {
        golem_runner::installer::PersistedInstall {
            fingerprint: fp,
            device_install_time: install_time,
            installed_version: None,
            installed_at: chrono::Utc::now(),
        }
    }

    fn info_present(install_time: Option<chrono::DateTime<chrono::Utc>>) -> golem_runner::installed_state::DeviceInstallInfo {
        golem_runner::installed_state::DeviceInstallInfo {
            installed: true,
            install_time,
            version: None,
        }
    }

    fn fp_git(rev: &str) -> golem_runner::fingerprint::Fingerprint {
        golem_runner::fingerprint::Fingerprint::Git {
            rev: rev.into(),
            porcelain: "x".into(),
        }
    }

    fn miss_reason(v: CacheVerdict) -> String {
        match v {
            CacheVerdict::Miss { reason } => reason,
            CacheVerdict::Hit { .. } => panic!("expected Miss, got Hit"),
        }
    }

    #[test]
    fn gate_decision_none_fingerprint_misses_with_reason() {
        let entry = make_entry(fp_git("a"), None);
        let v = gate_decision(
            Some(&entry),
            &golem_runner::fingerprint::Fingerprint::None,
            &info_present(None),
        );
        let r = miss_reason(v);
        assert!(r.contains("fingerprint unavailable"), "got: {r}");
    }

    #[test]
    fn gate_decision_no_entry_misses_with_reason() {
        let v = gate_decision(None, &fp_git("a"), &info_present(None));
        let r = miss_reason(v);
        assert!(r.contains("no prior cache entry"), "got: {r}");
    }

    #[test]
    fn gate_decision_fingerprint_mismatch_reports_both_sides() {
        let entry = make_entry(fp_git("aaaaaaaaaaa"), None);
        let v = gate_decision(Some(&entry), &fp_git("bbbbbbbbbbb"), &info_present(None));
        let r = miss_reason(v);
        assert!(r.contains("fingerprint changed"), "got: {r}");
        assert!(r.contains("git:aaaaaaa") && r.contains("git:bbbbbbb"),
            "miss reason SHALL show stored → current: {r}");
    }

    #[test]
    fn gate_decision_bundle_absent_misses_with_reason() {
        let entry = make_entry(fp_git("a"), None);
        let info = golem_runner::installed_state::DeviceInstallInfo::not_installed();
        let v = gate_decision(Some(&entry), &fp_git("a"), &info);
        let r = miss_reason(v);
        assert!(r.contains("no longer installed"), "got: {r}");
    }

    #[test]
    fn gate_decision_match_no_install_time_hits() {
        // When install-time isn't recorded on either side, fingerprint match
        // alone is sufficient.
        let entry = make_entry(fp_git("a"), None);
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(None));
        assert!(matches!(v, CacheVerdict::Hit { .. }));
    }

    #[test]
    fn gate_decision_install_time_match_hits() {
        let t = chrono::Utc::now();
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(t)));
        assert!(matches!(v, CacheVerdict::Hit { .. }));
    }

    #[test]
    fn gate_decision_install_time_drift_misses_with_external_reinstall_hint() {
        let t = chrono::Utc::now();
        let drifted = t + chrono::Duration::seconds(10);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(drifted)));
        let r = miss_reason(v);
        assert!(r.contains("install-time differs"), "got: {r}");
        assert!(r.contains("external reinstall"),
            "miss reason SHALL hint at external reinstall cause: {r}");
    }

    #[test]
    fn gate_decision_install_time_within_tolerance_hits() {
        let t = chrono::Utc::now();
        let close = t + chrono::Duration::seconds(1);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(close)));
        assert!(matches!(v, CacheVerdict::Hit { .. }), "1s drift SHALL be within tolerance");
    }

    // ---------------------------------------------------------------
    // is_transient_install_error — classifier for retry-on-transient
    // ---------------------------------------------------------------

    #[test]
    fn transient_classifier_matches_mach_308_text() {
        let err = "install script exited 204 for app-b on UDID:\n\
                   building GolemTestB (Debug) for UDID...\n\
                   installing ./build/...GolemTestB.app on UDID...\n\
                   An error was encountered processing the command \
                   (domain=NSMachErrorDomain, code=-308):\n\
                   The operation couldn’t be completed. (Mach error -308 \
                   - (ipc/mig) server died)";
        assert!(is_transient_install_error(err),
            "Mach -308 stderr SHALL be classified transient");
    }

    #[test]
    fn transient_classifier_matches_adb_device_offline() {
        let err = "install script exited 1: error: device offline";
        assert!(is_transient_install_error(err));
    }

    #[test]
    fn transient_classifier_rejects_genuine_failures() {
        // Compile errors, signing failures, missing schemes, etc. SHALL
        // NOT trigger a retry — they're real and would just waste 3s.
        assert!(!is_transient_install_error("error: scheme not found"));
        assert!(!is_transient_install_error("error: code signing required"));
        assert!(!is_transient_install_error("xcodebuild: error: nothing to build"));
        assert!(!is_transient_install_error(""));
    }

    #[test]
    fn gate_decision_hit_label_is_fingerprint_short_label() {
        let entry = make_entry(fp_git("abc1234567"), None);
        let v = gate_decision(Some(&entry), &fp_git("abc1234567"), &info_present(None));
        match v {
            CacheVerdict::Hit { label } => {
                assert!(label.contains("git:abc1234"),
                    "hit label SHALL be the source-fingerprint short label: {label}");
            }
            CacheVerdict::Miss { .. } => panic!("expected Hit"),
        }
    }

    // ---------------------------------------------------------------
    // 2. suite_stats counts passed flows
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_counts_passed() {
        let report = SuiteReport {
            flows: vec![passing_flow("a"), passing_flow("b"), passing_flow("c")],
            installs: Vec::new(),
            total_duration_ms: 100,
            started_at: None,
            finished_at: None,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.passed, 3);
    }

    // ---------------------------------------------------------------
    // 3. suite_stats counts failed flows
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_counts_failed() {
        let report = SuiteReport {
            flows: vec![failing_flow("a"), failing_flow("b")],
            installs: Vec::new(),
            total_duration_ms: 100,
            started_at: None,
            finished_at: None,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.failed, 2);
    }

    // ---------------------------------------------------------------
    // 4. suite_stats with mixed results
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_mixed_results() {
        let report = SuiteReport {
            flows: vec![
                passing_flow("a"),
                failing_flow("b"),
                passing_flow("c"),
                failing_flow("d"),
                passing_flow("e"),
            ],
            installs: Vec::new(),
            total_duration_ms: 500,
            started_at: None,
            finished_at: None,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 5);
        assert_eq!(stats.passed, 3);
        assert_eq!(stats.failed, 2);
    }

    // ---------------------------------------------------------------
    // 5. Empty suite produces empty report
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn empty_suite_produces_empty_report() {
        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let report = runner.run_suite(&[]).await.expect("run_suite");
        assert!(report.flows.is_empty());
    }

    // ---------------------------------------------------------------
    // 6. run_suite returns correct number of flow reports
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_suite_returns_correct_count() {
        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let paths = vec![
            PathBuf::from("login.test.toml"),
            PathBuf::from("checkout.test.toml"),
            PathBuf::from("signup.test.toml"),
        ];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        assert_eq!(report.flows.len(), 3);
    }

    // ---------------------------------------------------------------
    // 7. Suite duration is tracked
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn suite_duration_is_tracked() {
        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let paths = vec![PathBuf::from("a.test.toml")];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        // Duration should be a non-negative value (stub flows are instant,
        // so total_duration_ms will be 0 or very small).
        // We just verify the field is populated and the report succeeds.
        assert!(report.total_duration_ms < 1000);
    }

    // ---------------------------------------------------------------
    // 8. Seed is propagated to flow reports
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn seed_propagated_to_flow_reports() {
        let config = SuiteConfig {
            seed: Some(42),
            ..SuiteConfig::default()
        };
        let mut runner = SuiteRunner::new(config);
        let paths = vec![
            PathBuf::from("a.test.toml"),
            PathBuf::from("b.test.toml"),
        ];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        for flow in &report.flows {
            assert_eq!(flow.seed, Some(42));
        }
    }

    // ---------------------------------------------------------------
    // 9. SuiteStats with all passing
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_all_passing() {
        let report = SuiteReport {
            flows: vec![
                passing_flow("a"),
                passing_flow("b"),
                passing_flow("c"),
                passing_flow("d"),
            ],
            installs: Vec::new(),
            total_duration_ms: 200,
            started_at: None,
            finished_at: None,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.passed, 4);
        assert_eq!(stats.failed, 0);
    }

    // ---------------------------------------------------------------
    // 10. SuiteStats with all failing
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_all_failing() {
        let report = SuiteReport {
            flows: vec![
                failing_flow("a"),
                failing_flow("b"),
                failing_flow("c"),
            ],
            installs: Vec::new(),
            total_duration_ms: 300,
            started_at: None,
            finished_at: None,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 3);
        assert_eq!(stats.passed, 0);
        assert_eq!(stats.failed, 3);
    }

    // ---------------------------------------------------------------
    // 11. Flow names are extracted from file paths
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn flow_names_extracted_from_paths() {
        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let paths = vec![
            PathBuf::from("flows/auth/login.test.toml"),
            PathBuf::from("checkout.test.toml"),
        ];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        assert_eq!(report.flows[0].flow_name, "login.test");
        assert_eq!(report.flows[1].flow_name, "checkout.test");
    }

    // ---------------------------------------------------------------
    // 12. Empty suite stats
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_empty_suite() {
        let report = SuiteReport {
            flows: Vec::new(),
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.passed, 0);
        assert_eq!(stats.failed, 0);
    }

    // Parser + mixin expansion are covered by
    // `golem_orchestrator::plan::tests` (parse_one path) and
    // `golem_parser::mixin` unit tests — no need to re-test at this layer.

    // ---------------------------------------------------------------
    // 13. Missing flow file surfaces as a failed FlowReport
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn suite_reports_missing_flow_as_failed() {
        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let path = PathBuf::from("nonexistent_flow.test.toml");
        let report = runner.run_suite(&[path]).await.expect("run_suite");

        assert_eq!(
            report.flows.len(),
            1,
            "missing file SHALL produce exactly one flow report",
        );
        assert!(!report.flows[0].success, "report SHALL indicate failure");
        assert!(
            !report.flows[0].warnings.is_empty(),
            "warnings SHALL contain the parse error",
        );
    }

    // ---------------------------------------------------------------
    // 14. Invalid TOML surfaces as a failed FlowReport
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn suite_reports_invalid_toml_as_failed() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let flow_path = tmp.path().join("bad.test.toml");
        std::fs::write(&flow_path, "this is not [[[valid toml").expect("write bad flow");

        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let report = runner.run_suite(&[flow_path]).await.expect("run_suite");

        assert_eq!(
            report.flows.len(),
            1,
            "invalid TOML SHALL produce exactly one flow report",
        );
        assert!(!report.flows[0].success, "report SHALL indicate failure");
        assert!(
            !report.flows[0].warnings.is_empty(),
            "warnings SHALL contain the parse error",
        );
    }

    // ---------------------------------------------------------------
    // Install-cache-hit ranking: helper picks the warm device when multiple
    // free devices could match — saves re-running the install script.
    // ---------------------------------------------------------------
    fn test_device(name: &str, udid: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: udid.to_string(),
            platform: Platform::Ios,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 18,
            os_version: "18.0".to_string(),
            state: DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    fn test_slot(apps: &[&str]) -> DeviceSlot {
        DeviceSlot {
            platform: Some(Platform::Ios),
            os_version: None,
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
            apps: apps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn test_matrix_entry(app: &str, bundle: &str) -> InstallEntry {
        InstallEntry {
            platform: Platform::Ios,
            app_name: app.to_string(),
            bundle_id: bundle.to_string(),
            script_path: PathBuf::from("/tmp/noop.sh"),
            timeout_ms: 1000,
            device_constraints: Vec::new(),
        }
    }

    #[tokio::test]
    async fn with_resource_manager_shares_install_cache_across_runners() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        // Simulates how `OrchestratorServer` passes one cache into every
        // `handle_submit` call — prior submit's Succeeded entries must be
        // visible to the next runner's view.
        let rm = std::sync::Arc::new(
            golem_devices::resource_manager::ResourceManager::new(
                golem_devices::concurrency::ConcurrencyConfig::default(),
            ),
        );
        let shared = InstallCache::new();
        let r1 = SuiteRunner::with_resource_manager(SuiteConfig::default(), rm.clone(), shared.clone());
        let r2 = SuiteRunner::with_resource_manager(SuiteConfig::default(), rm.clone(), shared.clone());

        r1.install_cache
            .set(("udid-x".into(), "com.y".into()), InstallOutcome::Succeeded)
            .await;

        let seen = r2
            .install_cache
            .get(&("udid-x".into(), "com.y".into()))
            .await;
        assert!(
            matches!(seen, Some(InstallOutcome::Succeeded)),
            "an entry written via runner1 SHALL be visible on runner2 sharing the same InstallCache",
        );
    }

    #[tokio::test]
    async fn rank_prefers_device_with_cache_hit_for_slot_app() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(("udid-2".into(), "com.app".into()), InstallOutcome::Succeeded)
            .await;

        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(&free, Some(Platform::Ios), Some(&slot), Some(&cache), &matrix)
            .await;
        assert_eq!(
            pick.udid, "udid-2",
            "SHALL rank the sim with a Succeeded install cache entry above the cold one",
        );
    }

    #[tokio::test]
    async fn rank_without_cache_returns_first_candidate() {
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];
        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(&free, Some(Platform::Ios), Some(&slot), None, &matrix).await;
        assert_eq!(
            pick.udid, "udid-1",
            "SHALL fall back to input order when no cache is available",
        );
    }

    #[tokio::test]
    async fn rank_failed_cache_entry_does_not_count() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::FailedScript("nope".into()),
            )
            .await;

        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(&free, Some(Platform::Ios), Some(&slot), Some(&cache), &matrix)
            .await;
        assert_eq!(
            pick.udid, "udid-1",
            "FailedScript SHALL NOT count as a cache hit — first candidate wins the tie",
        );
    }

    #[tokio::test]
    async fn rank_picks_device_with_more_hits_for_multi_app_slot() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        // sim1 has one hit (app_a), sim2 has both (app_a + app_b).
        cache.set(("udid-1".into(), "com.a".into()), InstallOutcome::Succeeded).await;
        cache.set(("udid-2".into(), "com.a".into()), InstallOutcome::Succeeded).await;
        cache.set(("udid-2".into(), "com.b".into()), InstallOutcome::Succeeded).await;

        let slot = test_slot(&["app_a", "app_b"]);
        let matrix = vec![
            test_matrix_entry("app_a", "com.a"),
            test_matrix_entry("app_b", "com.b"),
        ];

        let pick = rank_by_install_cache(&free, Some(Platform::Ios), Some(&slot), Some(&cache), &matrix)
            .await;
        assert_eq!(
            pick.udid, "udid-2",
            "SHALL prefer the device with the higher cache-hit count across all slot apps",
        );
    }

    // ---------------------------------------------------------------
    // 15. Mixed good + bad paths: JIT scheduler emits one report per path
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn suite_mixes_parse_failures_with_unresolvable_flows() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let bad = tmp.path().join("bad.test.toml");
        std::fs::write(&bad, "broken [[[").expect("write bad");
        let missing = tmp.path().join("missing.test.toml");

        let mut runner = SuiteRunner::new(SuiteConfig::default());
        let report = runner
            .run_suite(&[bad, missing])
            .await
            .expect("run_suite SHALL succeed even when every flow fails to parse");
        assert_eq!(
            report.flows.len(),
            2,
            "one failed FlowReport per bad path SHALL appear in the suite report",
        );
        for flow in &report.flows {
            assert!(!flow.success);
        }
    }
}
