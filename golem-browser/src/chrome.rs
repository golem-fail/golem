use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chromiumoxide::detection::{default_executable, DetectionOptions};

/// Resolve the Chrome/Chromium binary golem will drive.
///
/// Delegates to chromiumoxide's own detection instead of keeping a private
/// table of install paths: the launcher resolves the executable this exact
/// way, and a preflight that disagreed could pass and then fail at launch (or
/// refuse a browser the launcher would have found).
pub fn locate() -> Result<PathBuf> {
    default_executable(DetectionOptions::default()).map_err(|e| {
        golem_events::coded(
            golem_events::FailureCode::HostBrowserMissing,
            anyhow!(
                "{e} — browse_* steps drive a real browser on this host. Install \
                 Chrome or Chromium, or point $CHROME at an existing binary."
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. Detection agrees with what a launch would use, and when it fails it
    //    fails as a host problem with an actionable message. Asserting on the
    //    outcome either way keeps the test meaningful on a Chrome-less box.
    #[test]
    fn locate_reports_a_binary_or_a_host_failure() {
        match locate() {
            Ok(path) => assert!(
                !path.as_os_str().is_empty(),
                "a detected browser SHALL have a path"
            ),
            Err(e) => {
                assert_eq!(
                    golem_events::extract_code(&e),
                    Some(golem_events::FailureCode::HostBrowserMissing)
                );
                assert!(
                    format!("{e:#}").contains("$CHROME"),
                    "a missing browser SHALL say how to fix it"
                );
            }
        }
    }
}
