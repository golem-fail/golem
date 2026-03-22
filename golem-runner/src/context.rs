use std::path::Path;

use crate::capture::CaptureConfig;

/// Flow-level context threaded through the execution pipeline.
pub struct ExecutionContext<'a> {
    pub flow_dir: &'a Path,
    pub project_root: &'a Path,
    pub capture_config: &'a CaptureConfig,
    pub flow_name: &'a str,
    pub block_name: Option<&'a str>,
    pub step_index: usize,
}

#[cfg(test)]
pub fn test_ctx(tmp: &std::path::Path) -> ExecutionContext<'_> {
    use std::sync::LazyLock;
    static DEFAULT_CAPTURE: LazyLock<CaptureConfig> = LazyLock::new(|| CaptureConfig {
        screenshot_on_failure: false,
        ..CaptureConfig::default()
    });
    ExecutionContext {
        flow_dir: tmp,
        project_root: tmp,
        capture_config: &DEFAULT_CAPTURE,
        flow_name: "test",
        block_name: None,
        step_index: 0,
    }
}
