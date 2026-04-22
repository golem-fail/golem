use std::path::{Path, PathBuf};
use std::time::Instant;

use std::sync::Arc;

use anyhow::Result;
use golem_devices::{DeviceInfo, DeviceState, Platform};
use golem_driver::android::AndroidDriver;
use golem_driver::ios::IosDriver;
use golem_driver::PlatformDriver;
use golem_orchestrator::{device_matches_slot, plan, DeviceSlot, FlowRun, InstallEntry};
use golem_parser::{parse_flow, FlowFile};
use golem_parser::mixin::expand_mixins;
use golem_report::{FlowReport, SuiteReport};
use golem_runner::capture::CaptureConfig;
use golem_runner::context::ExecutionContext;
use golem_runner::executor::execute_flow;
use golem_vars::VariableStore;

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

    /// Create a runner with a shared ResourceManager (for orchestrator mode).
    pub fn with_resource_manager(
        config: SuiteConfig,
        resource_mgr: std::sync::Arc<golem_devices::resource_manager::ResourceManager>,
    ) -> Self {
        Self {
            config,
            resource_mgr,
            event_forwarder: None,
            install_cache: golem_runner::installer::InstallCache::new(),
            install_matrix: Arc::new(Vec::new()),
            flow_paths: Arc::new(Vec::new()),
            flow_runs: Arc::new(Vec::new()),
            plan_event: None,
        }
    }

    /// Run a suite of flow files and return aggregated results.
    ///
    /// Run a suite of flows in parallel, gated by resource availability.
    ///
    /// All flows are spawned as concurrent tasks. The ResourceManager
    /// controls how many run simultaneously based on RAM and concurrency
    /// limits. Flows that can't allocate devices immediately will wait.
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
        )
        .await?;

        // Build the SuitePlanned event under --verbose. Emitted via the
        // event channel once it's set up — stream_human renders in
        // standalone mode; the orchestrator forwarder relays to clients
        // in server-for-client mode. Single code path, no direct eprintln.
        if self.config.verbose {
            self.plan_event = Some(build_suite_planned_event(&parsed));
        }

        self.install_matrix = Arc::new(parsed.install_matrix);
        self.flow_paths = Arc::new(flow_paths.to_vec());
        self.flow_runs = Arc::new(parsed.flow_runs);

        if flow_paths.len() == 1 {
            // Single flow — no need for suite-level parallelism.
            let (reports, installs) = self.run_single_flow(&flow_paths[0]).await;
            return Ok(SuiteReport {
                flows: reports,
                installs,
                total_duration_ms: start.elapsed().as_millis() as u64,
            });
        }

        // Multiple flows — run in parallel with shared ResourceManager.
        // Create a suite-level event channel so all flows stream through one
        // stream_human + accumulator, avoiding racing stderr writes.
        let resource_mgr = self.resource_mgr.clone();
        let (suite_tx, suite_rx) = golem_events::channel::event_channel();
        let verbose = self.config.verbose;
        let debug = self.config.debug;
        let stream_human_enabled = self.config.stream_human;
        if debug {
            golem_driver::set_debug(true);
        }

        // Suite-level stream_human (multi_device=true to get device prefixes).
        let human_handle = if stream_human_enabled {
            let human_rx = suite_rx.subscribe();
            Some(tokio::spawn(async move {
                golem_report::stream::stream_human(human_rx, verbose, true, debug).await;
            }))
        } else {
            None
        };

        // Suite-level accumulator.
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

        let mut handles = Vec::new();
        for path in flow_paths {
            let path = path.clone();
            // Clone the full parent config so sub-runners inherit every CLI
            // flag (no_clean, no_teardown, keep_devices, no_perf, verbose,
            // debug, …). Only stream_human is forced off because the
            // suite-level stream_human already consumes from suite_tx.
            let mut cfg = SuiteConfig {
                no_clean: self.config.no_clean,
                no_teardown: self.config.no_teardown,
                keep_devices: self.config.keep_devices,
                seed: self.config.seed,
                platform: self.config.platform,
                no_perf: self.config.no_perf,
                verbose: self.config.verbose,
                debug: self.config.debug,
                stream_human: false,
                start: self.config.start.clone(),
                vars: self.config.vars.clone(),
                output_dir: self.config.output_dir.clone(),
                no_results: self.config.no_results,
                project_root: self.config.project_root.clone(),
                project_apps: self.config.project_apps.clone(),
            };
            let install_cache = self.install_cache.clone();
            let install_matrix = self.install_matrix.clone();
            let flow_paths = self.flow_paths.clone();
            let flow_runs = self.flow_runs.clone();
            let rm = resource_mgr.clone();
            let suite_tx_clone = suite_tx.clone();

            handles.push(tokio::spawn(async move {
                cfg.stream_human = false;
                let mut runner = SuiteRunner::with_resource_manager(cfg, rm);
                runner.event_forwarder = Some(suite_tx_clone);
                runner.install_cache = install_cache;
                runner.install_matrix = install_matrix;
                runner.flow_paths = flow_paths;
                runner.flow_runs = flow_runs;
                runner.run_single_flow(&path).await
            }));
        }

        let mut flow_reports = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((reports, _installs)) => flow_reports.extend(reports),
                Err(e) => {
                    flow_reports.push(FlowReport {
                        flow_name: "unknown".to_string(),
                        success: false,
                        step_results: Vec::new(),
                        warnings: vec![format!("Task panicked: {e}")],
                        duration_ms: 0,
                        seed: self.config.seed,
                        screenshot_path: None,
                        device_name: None,
                        perf_snapshots: vec![],
                        skipped_reason: None,
                    });
                }
            }
        }

        // Emit suite summary and close suite channel.
        let passed = flow_reports.iter().filter(|r| r.success).count();
        let failed = flow_reports.iter().filter(|r| !r.success).count();
        suite_tx.emit(
            golem_events::DeviceId("suite".into()),
            golem_events::EventKind::SuiteFinished {
                duration_ms: start.elapsed().as_millis() as u64,
                passed,
                failed,
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
                if report.step_results.is_empty() && !acc_flow.step_results.is_empty() {
                    report.step_results = acc_flow.step_results.iter().map(|s| {
                        golem_report::StepReport {
                            global_step_index: s.global_step_index,
                            block_name: s.block_name.clone(),
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
                        }
                    }).collect();
                }
            }
        }

        Ok(SuiteReport {
            flows: flow_reports,
            installs: acc_report.installs,
            total_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Run a single flow using the shared ResourceManager.
    /// Returns (flow reports, install results) collected during the run.
    async fn run_single_flow(&mut self, path: &Path) -> (Vec<FlowReport>, Vec<golem_report::InstallReport>) {
        let rm = self.resource_mgr.clone();
        self.run_single_flow_with_resources(path, &rm).await
    }

    /// Install every `InstallEntry` from the suite's install matrix that is
    /// applicable to `(device, platform)`. Runs BEFORE companion registration
    /// so that `simctl install` / `adb install` can't tear down the xctest /
    /// instrumentation session mid-suite.
    ///
    /// An entry is applicable when:
    /// - its `platform` matches
    /// - the `(device.udid, entry.bundle_id)` pair is not already cached
    /// - the entry's `device_constraints` (from `[[flow.apps.devices]]`) don't
    ///   exclude this device — today only `device_type` (phone/tablet) is
    ///   checked; the device resolver typically prevents a mismatch upstream,
    ///   but we re-check here for safety when resolver is bypassed.
    ///
    /// Outcomes are written to `self.install_cache`; the per-flow install
    /// check will see `Succeeded` / `FailedScript` and skip re-running or
    /// skip the flow respectively.
    async fn preinstall_for_device(
        &self,
        device: &DeviceInfo,
        platform: Platform,
        event_tx: &golem_events::channel::EventSender,
    ) {
        let platform_str = platform.to_string();
        let target = format!(
            "{} ({}/v{}/{})",
            device.name, platform, device.os_major, device.device_type
        );
        // Use the same device_label format as per-flow emission so
        // stream_human's circled-number mapping stays stable across
        // preinstall and flow events for the same physical device.
        let device_label = format!("{platform}/{}", device.name);
        let emitter = golem_events::emitter::DeviceEmitter::new(
            event_tx.clone(),
            golem_events::DeviceId(device_label),
        );
        for entry in self.install_matrix.iter() {
            if entry.platform != platform {
                continue;
            }
            if !device_matches_entry_constraints(device, entry) {
                continue;
            }
            let key = (device.udid.clone(), entry.bundle_id.clone());
            // Fast-path cache check (optimisation).
            if self.install_cache.get(&key).await.is_some() {
                continue;
            }

            // Acquire project_lock BEFORE the authoritative cache re-check
            // to close the check-then-install race when multiple parallel
            // flow tasks share an install_cache (e.g. `golem run a.toml b.toml`
            // spawns 2 per-flow tasks, both preinstalling).
            let proj_lock = self
                .install_cache
                .project_lock(&self.config.project_root, &entry.script_path)
                .await;
            let _guard = proj_lock.lock().await;
            if self.install_cache.get(&key).await.is_some() {
                continue;
            }

            let result = golem_runner::installer::run_install_script(
                &entry.script_path,
                &self.config.project_root,
                &platform_str,
                &device.udid,
                &entry.bundle_id,
                &entry.app_name,
                entry.timeout_ms,
                &target,
                Some(&emitter),
            )
            .await;
            match result {
                Ok(()) => {
                    self.install_cache
                        .set(key, golem_runner::installer::InstallOutcome::Succeeded)
                        .await;
                }
                Err(e) => {
                    self.install_cache
                        .set(
                            key,
                            golem_runner::installer::InstallOutcome::FailedScript(format!("{e}")),
                        )
                        .await;
                }
            }
        }
    }

    /// Run a single flow file on all applicable platforms in parallel.
    ///
    /// Uses the ResourceManager to gate device allocation. If resources
    /// aren't available, waits until they are.
    async fn run_single_flow_with_resources(
        &mut self,
        path: &Path,
        resource_mgr: &golem_devices::resource_manager::ResourceManager,
    ) -> (Vec<FlowReport>, Vec<golem_report::InstallReport>) {
        let flow_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let start = Instant::now();

        let mut flow = match self.parse_and_expand(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("  Parse error: {e:#}");
                return (vec![FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: vec![format!("Parse/mixin error: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: self.config.seed,
                    screenshot_path: None,
                    device_name: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                }], Vec::new());
            }
        };

        // Merge project-level [[apps]] registry defaults into flow apps by name.
        // Flow-level fields override project-level ones.
        golem_orchestrator::merge_project_apps(&mut flow, &self.config.project_apps);

        // Detect target platforms from CLI override or flow's device constraints.
        let platforms = if let Some(p) = self.config.platform {
            vec![p]
        } else {
            detect_all_platforms(&flow)
        };

        // Create the event channel BEFORE device setup so stream_human can
        // render the Plan summary first (and eventually any setup-phase
        // events we migrate from eprintln). multi_device is estimated from
        // platform count — close enough for rendering purposes if one
        // platform later fails to resolve.
        let (event_tx, event_rx) = golem_events::channel::event_channel();
        let multi_device = platforms.len() > 1;
        let verbose = self.config.verbose;
        let debug = self.config.debug;
        if debug {
            golem_driver::set_debug(true);
        }

        let human_handle = if self.config.stream_human {
            let human_rx = event_rx.subscribe();
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
        let acc_rx = event_rx.subscribe();
        let acc_handle = tokio::spawn(async move {
            golem_report::accumulator::accumulate_events(acc_rx, &acc_clone).await;
        });

        let fwd_handle = if let Some(ref fwd) = self.event_forwarder {
            let fwd_rx = event_rx.subscribe();
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

        // Emit the Plan summary (if any) now that subscribers are attached —
        // BEFORE device discovery + install + companion setup. stream_human
        // renders for standalone; forwarder relays to orchestrator clients.
        if let Some(event) = self.plan_event.take() {
            event_tx.emit(golem_events::DeviceId("suite".into()), event);
        }

        drop(event_rx);

        // Read create_if_missing from flow options
        let create_if_missing = flow
            .flow
            .options
            .as_ref()
            .and_then(|o| o.create_if_missing)
            .unwrap_or(false);

        // Start the registration server for companion port allocation.
        let (reg_state, _reg_rx) = crate::registration::RegistrationState::new();
        let reg_port = crate::registration::start_registration_server(reg_state.clone())
            .await
            .unwrap_or(0);

        // Two-phase setup: (1) find devices + pre-install everywhere, (2)
        // start companions. Keeping installs first (before any companion is
        // up) prevents `simctl install` / `adb install` from tearing down a
        // running xctest / instrumentation session. Running ALL installs
        // before ANY companion also keeps iOS companions from sitting idle
        // long enough to hit xctest watchdog timeouts while other platforms
        // are still building.

        // Look up this flow's FlowRun slots once so platform picks honour
        // `os_version` + other slot fields (device_type, physical, name).
        // Fan-out (`:latest:N`, type lists) produces multiple FlowRuns per
        // flow_idx; the current execute loop still treats one platform as
        // one device — we pick the first slot matching each platform.
        // Multi-run fan-out is handled by the Dynamic JIT Scheduler (roadmap).
        let flow_idx = self
            .flow_paths
            .iter()
            .position(|p| p == path);

        // Phase 1: discover devices + pre-install apps.
        let mut installed: Vec<(DeviceInfo, Platform)> = Vec::new();
        for platform in &platforms {
            let slot: Option<&DeviceSlot> = flow_idx.and_then(|idx| {
                self.flow_runs
                    .iter()
                    .filter(|r| r.flow_idx == idx)
                    .flat_map(|r| r.slots.iter())
                    .find(|s| s.platform == *platform)
            });
            match find_available_device(*platform, slot, resource_mgr, create_if_missing).await {
                Ok(device) => {
                    // Section header — minimal value, gated behind --debug.
                    // The [install app] + Companion lines that follow already
                    // name the platform via context.
                    if self.config.debug {
                        eprintln!("  Platform: {platform}");
                    }
                    self.preinstall_for_device(&device, *platform, &event_tx).await;
                    installed.push((device, *platform));
                }
                Err(e) => {
                    eprintln!("  [devices] no {platform} available: {e:#}");
                }
            }
        }

        // Phase 2: start companions + allocate + health-check.
        let mut device_setups = Vec::new();
        for (device, platform) in installed {
            // Try to find an existing companion first (legacy scan)
            let existing_port = find_or_allocate_port(&device, platform).await.ok();
            let port = if let Some(p) = existing_port {
                let client = golem_driver::common::CompanionClient::new(p);
                if let Ok(health) = client.check_health().await {
                    if health.version == env!("CARGO_PKG_VERSION") {
                        p
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                0
            };

            let port = if port > 0 {
                port
            } else {
                match ensure_companion_with_reg(&device, platform, reg_port, &reg_state).await {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("  [companion] failed for {platform}: {e:#}");
                        continue;
                    }
                }
            };

            // Wait for resource allocation (RAM + concurrency limit)
            let alloc_deadline = tokio::time::Instant::now()
                + std::time::Duration::from_secs(1200);
            let mut printed_waiting = false;
            loop {
                match resource_mgr.try_allocate(&device, port) {
                    Ok(()) => break,
                    Err(e) => {
                        if tokio::time::Instant::now() >= alloc_deadline {
                            eprintln!("  [resources] timed out waiting for {platform}: {e:#}");
                            break;
                        }
                        if !printed_waiting {
                            eprintln!("  [resources] waiting for {platform}...");
                            printed_waiting = true;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }

            let client = golem_driver::common::CompanionClient::new(port);
            match client.check_health().await {
                Ok(health) => {
                    eprintln!(
                        "  [companion] ready — {} v{} on {} ({})",
                        health.platform, health.version, health.device_name, health.os_version
                    );
                    device_setups.push((device, platform, port));
                }
                Err(e) => {
                    eprintln!("  Companion failed for {platform}: {e:#}");
                    resource_mgr.release(&device.udid);
                }
            }
        }

        if device_setups.is_empty() {
            return (vec![FlowReport {
                flow_name,
                success: false,
                step_results: Vec::new(),
                warnings: vec!["No devices available for any target platform".to_string()],
                duration_ms: start.elapsed().as_millis() as u64,
                seed: self.config.seed,
                screenshot_path: None,
                device_name: None,
                perf_snapshots: vec![],
                skipped_reason: None,
            }], Vec::new());
        }

        // Track allocated device UDIDs for release after execution.
        let allocated_udids: Vec<String> = device_setups.iter().map(|(d, _, _)| d.udid.clone()).collect();

        // Spawn parallel execution tasks — one per device.
        // Shared failure barrier: when one device fails, others stop at the same step.
        // MUST stay per-flow: step counts only compare within a single flow.
        // See `golem-runner/src/barrier.rs` module docs.
        let barrier = golem_runner::barrier::FailureBarrier::new();
        let mut handles = Vec::new();
        for (device, platform, port) in device_setups {
            let flow = flow.clone();
            let flow_name = flow_name.clone();
            let flow_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let seed = self.config.seed;
            let barrier = barrier.clone();
            let no_perf = self.config.no_perf;
            let start_block = self.config.start.clone();
            let cli_vars = self.config.vars.clone();
            let output_dir = self.config.output_dir.clone();
            let no_results = self.config.no_results;
            let install_cache = self.install_cache.clone();
            let project_root = self.config.project_root.clone();
            let tx = event_tx.clone();

            handles.push(tokio::spawn(async move {
                run_flow_on_device(flow, flow_name, flow_dir, device, platform, port, seed, start_block, cli_vars, output_dir, no_results, install_cache, project_root, barrier, no_perf, Some(tx)).await
            }));
        }

        // Collect results from all spawned tasks.
        let mut reports = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(report) => reports.push(report),
                Err(e) => {
                    reports.push(FlowReport {
                        flow_name: flow_name.clone(),
                        success: false,
                        step_results: Vec::new(),
                        warnings: vec![format!("Task panicked: {e}")],
                        duration_ms: start.elapsed().as_millis() as u64,
                        seed: self.config.seed,
                        screenshot_path: None,
                        device_name: None,
                        perf_snapshots: vec![],
                        skipped_reason: None,
                    });
                }
            }
        }

        // Emit suite summary before closing channel — but only when this is
        // the top-level runner. When an event_forwarder is present, a parent
        // (multi-flow suite or orchestrator client) will emit SuiteFinished.
        if self.event_forwarder.is_none() {
            let passed = reports.iter().filter(|r| r.success).count();
            let failed = reports.iter().filter(|r| !r.success).count();
            event_tx.emit(
                golem_events::DeviceId("suite".into()),
                golem_events::EventKind::SuiteFinished {
                    duration_ms: start.elapsed().as_millis() as u64,
                    passed,
                    failed,
                },
            );
        }
        drop(event_tx);
        if let Some(h) = human_handle { let _ = h.await; }
        let _ = acc_handle.await;
        if let Some(h) = fwd_handle { let _ = h.await; }

        // Merge step data from accumulator into flow reports.
        let acc_report = {
            let taken = std::mem::replace(
                &mut *accumulator.lock().await,
                golem_report::accumulator::ReportAccumulator::new(),
            );
            taken.into_suite_report()
        };
        for report in &mut reports {
            if let Some(acc_flow) = acc_report.flows.iter().find(|f| {
                f.device_name.as_deref() == report.device_name.as_deref()
                    && f.flow_name == report.flow_name
            }) {
                if report.step_results.is_empty() && !acc_flow.step_results.is_empty() {
                    report.step_results = acc_flow.step_results.iter().map(|s| {
                        golem_report::StepReport {
                            global_step_index: s.global_step_index,
                            block_name: s.block_name.clone(),
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
                        }
                    }).collect();
                }
            }
        }

        // Release all allocated devices back to the ResourceManager.
        for udid in &allocated_udids {
            resource_mgr.release(udid);
        }

        (reports, acc_report.installs)
    }

    /// Read, parse, and expand mixins in a flow file.
    ///
    /// Returns the fully-expanded [`FlowFile`] ready for execution.
    fn parse_and_expand(&self, path: &Path) -> Result<FlowFile> {
        let content = std::fs::read_to_string(path)?;
        let mut flow = parse_flow(&content)?;

        let flow_dir = path.parent().unwrap_or(Path::new("."));
        let project_root = self.config.project_root.as_path();

        for block in &mut flow.block {
            block.steps = expand_mixins(&block.steps, flow_dir, project_root)?;
        }

        Ok(flow)
    }
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
        if let Some(phys) = c.physical {
            if device.physical != phys {
                return false;
            }
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

    // If exactly one companion for this platform, assume it's ours
    if platform_companions.len() == 1 {
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
) -> Result<u16> {
    eprintln!("  [companion] not running — starting...");
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

    // Wait for the companion to register (up to 60s)
    let mut rx = reg_state.subscribe();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        tokio::select! {
            msg = rx.recv() => {
                if let Ok(registered_id) = msg {
                    if let Some(comp) = reg_state.get(&registered_id) {
                        // For Android, set up ADB forward for the assigned port
                        if platform == Platform::Android {
                            let fwd = golem_devices::lifecycle::port_forward_command(device, comp.port);
                            let _ = golem_devices::lifecycle::run_command_public(&fwd, "port forward").await;
                        }
                        // Wait briefly for the companion to start serving after registration
                        let client = golem_driver::common::CompanionClient::new(comp.port);
                        let _ = client.wait_for_health(std::time::Duration::from_secs(15)).await;
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

/// Detect ALL platforms referenced in the flow's device constraints.
/// Returns a deduplicated list. Defaults to `[Platform::Ios]` when no
/// constraints are specified.
fn detect_all_platforms(flow: &FlowFile) -> Vec<Platform> {
    let mut platforms = Vec::new();
    for app in &flow.flow.apps {
        for constraint in &app.devices {
            if let Some(ref os) = constraint.os {
                for os_str in os.to_vec() {
                    let p = if os_str.starts_with("android") {
                        Platform::Android
                    } else {
                        Platform::Ios
                    };
                    if !platforms.contains(&p) {
                        platforms.push(p);
                    }
                }
            }
        }
    }
    if platforms.is_empty() {
        platforms.push(Platform::Ios);
    }
    platforms
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
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
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
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
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
                    perf_snapshots: vec![],
                    skipped_reason: Some(reason),
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
            // Serialise install script runs that share a (project_root, script).
            // Concurrent tauri ios + android builds in one src-tauri/ collide
            // on cargo target-dir locks and WS IPC → ECONNREFUSED.
            let proj_lock = install_cache.project_lock(&project_root, &script_path).await;
            let _guard = proj_lock.lock().await;
            // Re-check cache inside the lock — another parallel flow task
            // may have installed this (device, bundle) while we waited.
            if matches!(
                install_cache.get(&key).await,
                Some(golem_runner::installer::InstallOutcome::Succeeded)
            ) {
                continue;
            }
            let target = format!(
                "{} ({}/v{}/{})",
                device.name, platform, device.os_major, device.device_type
            );
            let result = golem_runner::installer::run_install_script(
                &script_path,
                &project_root,
                platform_str,
                &device.udid,
                &bundle_for_install,
                &app.name,
                timeout_ms,
                &target,
                device_emitter.as_ref(),
            ).await;
            match result {
                Ok(()) => {
                    install_cache.set(key, golem_runner::installer::InstallOutcome::Succeeded).await;
                }
                Err(e) => {
                    let err_str = format!("{e}");
                    install_cache.set(key, golem_runner::installer::InstallOutcome::FailedScript(err_str.clone())).await;
                    let reason = format!("install_script failed for {} on {device_name}: {err_str}", bundle_for_install);
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
                        perf_snapshots: vec![],
                        skipped_reason: Some(reason),
                    };
                }
            }
        }
    }

    ctx.emit(golem_events::EventKind::FlowStarted { flow_name: flow_name.clone() });
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
                perf_snapshots: result.perf_snapshots,
                skipped_reason: None,
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
                perf_snapshots: vec![],
                skipped_reason: None,
            }
        }
    }
}

/// Discover ALL devices for the given platform (booted and shutdown).
async fn discover_all_devices(platform: Platform) -> Result<Vec<DeviceInfo>> {
    match platform {
        Platform::Ios => {
            golem_devices::ios::discover_ios_devices().await
        }
        Platform::Android => {
            let output = tokio::process::Command::new("adb")
                .args(["devices"])
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut devices = Vec::new();
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 && parts[1] == "device" {
                    let serial = parts[0].to_string();
                    // Query the actual OS version via `adb shell getprop`.
                    // Hardcoding `os_major: 0` would leak into display labels
                    // like `android/v0/phone` and break version-based slot
                    // matching in the Plan phase.
                    let sdk = tokio::process::Command::new("adb")
                        .args(["-s", &serial, "shell", "getprop", "ro.build.version.sdk"])
                        .output()
                        .await
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_default();
                    let release = tokio::process::Command::new("adb")
                        .args(["-s", &serial, "shell", "getprop", "ro.build.version.release"])
                        .output()
                        .await
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_default();
                    let os_major = sdk.parse::<u32>().unwrap_or(0);
                    devices.push(DeviceInfo {
                        name: serial.clone(),
                        udid: serial,
                        platform: Platform::Android,
                        device_type: golem_devices::DeviceType::Phone,
                        os_major,
                        os_version: release,
                        state: DeviceState::Booted,
                        physical: false,
                        playstore: false,
                        screen_width: None,
                        screen_height: None,
                        screen_scale: None,
                        last_booted: None,
                        runtime_id: None,
                        device_type_id: None,
                    });
                }
            }
            Ok(devices)
        }
    }
}

/// Find the best available device for a platform, honouring slot
/// requirements if provided (os_version, device_type, physical, name).
///
/// Priority:
/// 1. Free booted device matching slot → return immediately
/// 2. No matching booted → auto-boot the best shutdown device that
///    matches the slot (highest `os_major` tie-break among equally-matching
///    candidates, which naturally honours `Exact(N)` after filter)
/// 3. No compatible devices at all → auto-create if `create_if_missing`
/// 4. All matching booted busy → wait up to 20 minutes
/// 5. No compatible devices and `create_if_missing` false → fail
///
/// When `slot` is `None` (direct test harness, no plan phase) we fall
/// back to the pre-slot behaviour: match by platform only.
async fn find_available_device(
    platform: Platform,
    slot: Option<&DeviceSlot>,
    resource_mgr: &golem_devices::resource_manager::ResourceManager,
    create_if_missing: bool,
) -> Result<DeviceInfo> {
    let all_devices = discover_all_devices(platform).await?;

    let compatible: Vec<&DeviceInfo> = all_devices
        .iter()
        .filter(|d| d.platform == platform)
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

    // Step 1: Try to find a free booted device
    if !booted.is_empty() {
        // Booted count already reported via SuitePlanned/device_availability.
        for device in &booted {
            if resource_mgr.port_for(&device.udid).is_none() {
                return Ok((*device).clone());
            }
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
            .unwrap_or_else(|| platform.to_string());
        eprintln!(
            "  [devices] no booted {platform} — booting {} to satisfy {shape}...",
            best.name
        );
        golem_devices::lifecycle::boot_device(best).await?;
        return Ok(DeviceInfo {
            state: DeviceState::Booted,
            ..(*best).clone()
        });
    }

    // Step 3: No compatible devices at all — auto-create or fail
    if compatible.is_empty() {
        if create_if_missing {
            eprintln!("  [devices] no {platform} device found — creating one...");
            let config = golem_devices::concurrency::ConcurrencyConfig::default();
            return golem_devices::lifecycle::auto_create_device(
                platform,
                golem_devices::DeviceType::Phone,
                &config,
            ).await;
        } else {
            anyhow::bail!(
                "No {platform} devices found. Use create_if_missing = true to auto-create, \
                 or boot a simulator/emulator manually."
            );
        }
    }

    // Step 4: All booted devices are busy — wait for one to free up
    let timeout = std::time::Duration::from_secs(1200); // 20 minutes
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        for device in &booted {
            if resource_mgr.port_for(&device.udid).is_none() {
                return Ok((*device).clone());
            }
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "Timed out waiting for a free {platform} device (all {} are in use)",
                booted.len()
            );
        }

        eprintln!("  [devices] all {platform} devices busy, waiting...");
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
        passed: report.flows.iter().filter(|f| f.success).count(),
        failed: report.flows.iter().filter(|f| !f.success).count(),
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
            perf_snapshots: vec![],
            skipped_reason: None,
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
            perf_snapshots: vec![],
            skipped_reason: None,
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
        let runner = SuiteRunner::new(SuiteConfig::default());
        let report = runner.run_suite(&[]).await.expect("run_suite");
        assert!(report.flows.is_empty());
    }

    // ---------------------------------------------------------------
    // 6. run_suite returns correct number of flow reports
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_suite_returns_correct_count() {
        let runner = SuiteRunner::new(SuiteConfig::default());
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
        let runner = SuiteRunner::new(SuiteConfig::default());
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
        let runner = SuiteRunner::new(config);
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
        let runner = SuiteRunner::new(SuiteConfig::default());
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
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.passed, 0);
        assert_eq!(stats.failed, 0);
    }

    // ---------------------------------------------------------------
    // 13. parse_and_expand reads flow and expands mixins
    // ---------------------------------------------------------------
    #[test]
    fn parse_and_expand_reads_flow_file() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let flow_toml = r#"
[flow]
name = "basic flow"

[[block]]
name = "block1"
steps = [
  { action = "tap", text = "Hello" },
]
"#;
        let flow_path = tmp.path().join("basic.test.toml");
        std::fs::write(&flow_path, flow_toml).expect("write flow");

        let runner = SuiteRunner::new(SuiteConfig::default());
        let flow = runner
            .parse_and_expand(&flow_path)
            .expect("parse_and_expand SHALL succeed");

        assert_eq!(flow.flow.name, "basic flow");
        assert_eq!(flow.block.len(), 1);
        assert_eq!(flow.block[0].steps.len(), 1);
        assert_eq!(flow.block[0].steps[0].action, "tap");
    }

    // ---------------------------------------------------------------
    // 14. parse_and_expand expands load_mixin steps
    // ---------------------------------------------------------------
    #[test]
    fn parse_and_expand_expands_mixins() {
        let tmp = tempfile::tempdir().expect("temp dir");

        // Create mixin file
        let mixins_dir = tmp.path().join("__mixins__");
        std::fs::create_dir_all(&mixins_dir).expect("create mixins dir");
        std::fs::write(
            mixins_dir.join("login.toml"),
            r#"
[[step]]
action = "type"
id = "email_field"
text = "{{email}}"

[[step]]
action = "tap"
text = "Submit"
"#,
        )
        .expect("write mixin");

        // Create flow file referencing the mixin
        let flow_toml = r#"
[flow]
name = "mixin flow"

[[block]]
name = "login"
steps = [
  { action = "load_mixin", mixin = "login" },
  { action = "screenshot" },
]
"#;
        let flow_path = tmp.path().join("mixin_flow.test.toml");
        std::fs::write(&flow_path, flow_toml).expect("write flow");

        let runner = SuiteRunner::new(SuiteConfig::default());
        let flow = runner
            .parse_and_expand(&flow_path)
            .expect("parse_and_expand with mixins SHALL succeed");

        // The load_mixin step should be replaced by the mixin's 2 steps + the screenshot step
        assert_eq!(
            flow.block[0].steps.len(),
            3,
            "load_mixin SHALL be expanded to the mixin's steps"
        );
        assert_eq!(flow.block[0].steps[0].action, "type");
        assert_eq!(flow.block[0].steps[1].action, "tap");
        assert_eq!(flow.block[0].steps[2].action, "screenshot");
    }

    // ---------------------------------------------------------------
    // 15. run_single_flow fails gracefully for missing file
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_single_flow_fails_for_missing_file() {
        let runner = SuiteRunner::new(SuiteConfig::default());
        let path = PathBuf::from("nonexistent_flow.test.toml");
        let (reports, _installs) = runner.run_single_flow(&path).await;

        assert_eq!(reports.len(), 1, "missing file SHALL produce exactly one report");
        assert!(!reports[0].success, "report SHALL indicate failure for missing file");
        assert!(
            !reports[0].warnings.is_empty(),
            "warnings SHALL contain the parse error"
        );
    }

    // ---------------------------------------------------------------
    // 16. run_single_flow fails gracefully for invalid TOML
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_single_flow_fails_for_invalid_toml() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let flow_path = tmp.path().join("bad.test.toml");
        std::fs::write(&flow_path, "this is not [[[valid toml").expect("write bad flow");

        let runner = SuiteRunner::new(SuiteConfig::default());
        let (reports, _installs) = runner.run_single_flow(&flow_path).await;

        assert_eq!(reports.len(), 1, "invalid TOML SHALL produce exactly one report");
        assert!(!reports[0].success, "report SHALL indicate failure for invalid TOML");
        assert!(
            !reports[0].warnings.is_empty(),
            "warnings SHALL contain the parse error"
        );
    }
}
