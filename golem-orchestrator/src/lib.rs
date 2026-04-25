//! Suite orchestration: Plan → Execute model.
//!
//! The Plan phase runs once at suite start. It parses all flows, merges
//! project `[[apps]]` defaults, detects target platforms, and produces:
//!
//! - `flows: Vec<ParsedFlow>` — parsed + merged flow files
//! - `flow_runs: Vec<FlowRun>` — one run per (flow × target platform)
//! - `install_matrix: Vec<InstallEntry>` — apps actually referenced by some
//!   flow, keyed by (platform, app_name). Apps in `golem.toml [[apps]]` that
//!   no flow references are dropped entirely.
//!
//! The Execute phase (in `golem-cli::suite`) consumes a `ParsedSuite`,
//! allocates devices per `FlowRun`, runs scoped pre-install from the
//! `install_matrix`, ensures a companion, and executes the flow.
//!
//! Coverage groups (`CoverageGroup`) wire execute-time adaptive
//! strategies (`One`, `Smart`) into the JIT FlowRun scheduler. Each group
//! holds a flat tick-box pool; `FlowRun.coverage_group` points at the
//! group and `FlowRun.covers_boxes` pre-declares which pool indices the
//! run will tick on success. The scheduler gates every spawn against a
//! shared progress tracker.
//!
//! Roadmap items this crate accommodates without re-refactor:
//! - Cross-process install cache dedup → `InstallCache` trait in
//!   `golem-runner::installer` can grow a persistent backend

pub mod coverage;
pub mod install_matrix;
pub mod plan;

pub use coverage::{pick_best_covering, set_cover_greedy, CoverageStrategy};
pub use install_matrix::{build_install_matrix, InstallEntry};
pub use plan::{
    describe_slot, device_matches_slot, merge_project_apps, plan, shape_label, CoverageGroup,
    DeviceSlot, FlowRun, ParseFailure, ParsedFlow, ParsedSuite,
};
