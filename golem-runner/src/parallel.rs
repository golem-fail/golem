use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

/// Result of executing a block across multiple devices.
pub struct ParallelBlockResult {
    pub device_results: Vec<DeviceBlockResult>,
}

/// Result of executing a block on a single device.
pub struct DeviceBlockResult {
    pub device_id: String,
    pub success: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

impl ParallelBlockResult {
    /// Returns `true` if every device in the block succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.device_results.iter().all(|r| r.success)
    }

    /// Returns the device IDs that failed.
    pub fn failed_devices(&self) -> Vec<&str> {
        self.device_results
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.device_id.as_str())
            .collect()
    }
}

/// Execute a block's steps on multiple devices in parallel.
///
/// Each device gets its own tokio task, bounded by a [`Semaphore`] with
/// `max_concurrency` permits. The function returns only after **all** tasks
/// complete, making each call an implicit barrier between blocks.
pub async fn execute_block_parallel<F, Fut>(
    device_ids: &[String],
    max_concurrency: usize,
    task_fn: F,
) -> ParallelBlockResult
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<String>>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let task_fn = Arc::new(task_fn);

    let mut handles = Vec::new();

    for device_id in device_ids {
        let sem = semaphore.clone();
        let id = device_id.clone();
        let func = task_fn.clone();

        let handle = tokio::spawn(async move {
            let _permit = match sem.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    return DeviceBlockResult {
                        device_id: id,
                        success: false,
                        error: Some("Semaphore closed".to_string()),
                        warnings: Vec::new(),
                    };
                }
            };
            match func(id.clone()).await {
                Ok(warnings) => DeviceBlockResult {
                    device_id: id,
                    success: true,
                    error: None,
                    warnings,
                },
                Err(e) => DeviceBlockResult {
                    device_id: id,
                    success: false,
                    error: Some(e.to_string()),
                    warnings: Vec::new(),
                },
            }
        });
        handles.push(handle);
    }

    let mut device_results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => device_results.push(result),
            Err(e) => device_results.push(DeviceBlockResult {
                device_id: "unknown".to_string(),
                success: false,
                error: Some(format!("Task panicked: {e}")),
                warnings: Vec::new(),
            }),
        }
    }

    ParallelBlockResult { device_results }
}

/// Default max concurrency when not specified by the user.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    // ---------------------------------------------------------------
    // 1. Single device executes successfully
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn single_device_executes_successfully() {
        let result = execute_block_parallel(
            &["device1".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |_id| async { Ok(Vec::new()) },
        )
        .await;

        assert_eq!(result.device_results.len(), 1);
        assert!(result.device_results[0].success);
        assert_eq!(result.device_results[0].device_id, "device1");
        assert!(result.device_results[0].error.is_none());
    }

    // ---------------------------------------------------------------
    // 2. Multiple devices execute in parallel
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn multiple_devices_execute_in_parallel() {
        let started = Arc::new(AtomicUsize::new(0));
        let started_clone = started.clone();

        let result = execute_block_parallel(
            &[
                "d1".to_string(),
                "d2".to_string(),
                "d3".to_string(),
            ],
            4,
            move |_id| {
                let started = started_clone.clone();
                async move {
                    started.fetch_add(1, Ordering::SeqCst);
                    // Brief yield to allow other tasks to start
                    tokio::task::yield_now().await;
                    Ok(Vec::new())
                }
            },
        )
        .await;

        assert_eq!(result.device_results.len(), 3);
        assert_eq!(started.load(Ordering::SeqCst), 3);
        assert!(result.all_succeeded());
    }

    // ---------------------------------------------------------------
    // 3. All devices succeed -> all_succeeded() is true
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn all_succeeded_returns_true_when_all_pass() {
        let result = execute_block_parallel(
            &["a".to_string(), "b".to_string(), "c".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |_id| async { Ok(Vec::new()) },
        )
        .await;

        assert!(result.all_succeeded());
        assert!(result.failed_devices().is_empty());
    }

    // ---------------------------------------------------------------
    // 4. One device fails -> all_succeeded() is false, failed_devices()
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn one_device_fails_detected_correctly() {
        let result = execute_block_parallel(
            &["ok1".to_string(), "fail1".to_string(), "ok2".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |id| async move {
                if id == "fail1" {
                    Err(anyhow::anyhow!("device failed"))
                } else {
                    Ok(Vec::new())
                }
            },
        )
        .await;

        assert!(!result.all_succeeded());
        let failed = result.failed_devices();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0], "fail1");

        // Verify the error message is captured
        let fail_result = result
            .device_results
            .iter()
            .find(|r| r.device_id == "fail1")
            .expect("should find fail1");
        assert!(
            fail_result
                .error
                .as_ref()
                .is_some_and(|e| e.contains("device failed"))
        );
    }

    // ---------------------------------------------------------------
    // 5. Semaphore limits concurrency
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn semaphore_limits_concurrency() {
        let max_concurrency = 2;
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let concurrent_clone = concurrent.clone();
        let peak_clone = peak.clone();

        let devices: Vec<String> = (0..6).map(|i| format!("device{i}")).collect();

        let result = execute_block_parallel(&devices, max_concurrency, move |_id| {
            let concurrent = concurrent_clone.clone();
            let peak = peak_clone.clone();
            async move {
                let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                // Update peak
                loop {
                    let old_peak = peak.load(Ordering::SeqCst);
                    if current <= old_peak {
                        break;
                    }
                    match peak.compare_exchange_weak(
                        old_peak,
                        current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                }

                // Hold the permit for a bit so concurrency is observable
                tokio::time::sleep(Duration::from_millis(50)).await;

                concurrent.fetch_sub(1, Ordering::SeqCst);
                Ok(Vec::new())
            }
        })
        .await;

        assert!(result.all_succeeded());
        assert_eq!(result.device_results.len(), 6);
        let observed_peak = peak.load(Ordering::SeqCst);
        assert!(
            observed_peak <= max_concurrency,
            "Peak concurrency {observed_peak} should not exceed {max_concurrency}"
        );
    }

    // ---------------------------------------------------------------
    // 6. All devices fail -> all results contain errors
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn all_devices_fail_all_have_errors() {
        let result = execute_block_parallel(
            &["a".to_string(), "b".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |id| async move { Err(anyhow::anyhow!("{id} broke")) },
        )
        .await;

        assert!(!result.all_succeeded());
        assert_eq!(result.failed_devices().len(), 2);
        for r in &result.device_results {
            assert!(!r.success);
            assert!(r.error.is_some());
        }
    }

    // ---------------------------------------------------------------
    // 7. Empty device list -> empty results
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn empty_device_list_returns_empty_results() {
        let result = execute_block_parallel(
            &[],
            DEFAULT_MAX_CONCURRENCY,
            |_id| async { Ok(Vec::new()) },
        )
        .await;

        assert!(result.device_results.is_empty());
        assert!(result.all_succeeded()); // vacuously true
        assert!(result.failed_devices().is_empty());
    }

    // ---------------------------------------------------------------
    // 8. Warnings are collected from successful tasks
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn warnings_collected_from_successful_tasks() {
        let result = execute_block_parallel(
            &["d1".to_string(), "d2".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |id| async move {
                Ok(vec![format!("warning from {id}")])
            },
        )
        .await;

        assert!(result.all_succeeded());
        for r in &result.device_results {
            assert_eq!(r.warnings.len(), 1);
            assert!(r.warnings[0].contains(&r.device_id));
        }
    }

    // ---------------------------------------------------------------
    // 9. Task panic is handled gracefully
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn task_panic_handled_gracefully() {
        let result = execute_block_parallel(
            &["panicker".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |_id| async move {
                panic!("intentional test panic");
            },
        )
        .await;

        assert!(!result.all_succeeded());
        assert_eq!(result.device_results.len(), 1);
        assert!(!result.device_results[0].success);
        let error = result.device_results[0]
            .error
            .as_ref()
            .expect("should have error");
        assert!(
            error.contains("panicked"),
            "error should mention panic: {error}"
        );
    }

    // ---------------------------------------------------------------
    // 10. Default max concurrency is 4
    // ---------------------------------------------------------------
    #[test]
    fn default_max_concurrency_is_four() {
        assert_eq!(DEFAULT_MAX_CONCURRENCY, 4);
    }

    // ---------------------------------------------------------------
    // 11. Device IDs are correctly assigned in results
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn device_ids_correctly_assigned() {
        let devices = vec![
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
        ];
        let result = execute_block_parallel(
            &devices,
            DEFAULT_MAX_CONCURRENCY,
            |_id| async { Ok(Vec::new()) },
        )
        .await;

        let mut result_ids: Vec<&str> = result
            .device_results
            .iter()
            .map(|r| r.device_id.as_str())
            .collect();
        result_ids.sort();
        assert_eq!(result_ids, vec!["alpha", "beta", "gamma"]);
    }

    // ---------------------------------------------------------------
    // 12. failed_devices() reports every failing id when several fail
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn failed_devices_reports_all_failing_ids() {
        let result = execute_block_parallel(
            &[
                "ok1".to_string(),
                "bad1".to_string(),
                "ok2".to_string(),
                "bad2".to_string(),
            ],
            DEFAULT_MAX_CONCURRENCY,
            |id| async move {
                if id.starts_with("bad") {
                    Err(anyhow::anyhow!("{id} failed"))
                } else {
                    Ok(Vec::new())
                }
            },
        )
        .await;

        let mut failed = result.failed_devices();
        failed.sort();
        assert_eq!(
            failed,
            vec!["bad1", "bad2"],
            "failed_devices SHALL list exactly the failing device ids"
        );
        assert!(!result.all_succeeded());
    }

    // ---------------------------------------------------------------
    // 13. A failed task carries no warnings (error path clears them)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn failed_task_has_empty_warnings() {
        let result = execute_block_parallel(
            &["boom".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |_id| async move { Err::<Vec<String>, _>(anyhow::anyhow!("nope")) },
        )
        .await;

        assert_eq!(result.device_results.len(), 1);
        assert!(
            result.device_results[0].warnings.is_empty(),
            "a failed device SHALL carry no warnings"
        );
    }

    // ---------------------------------------------------------------
    // 14. Mixed run: success keeps warnings, failure keeps error
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn mixed_run_preserves_warnings_and_errors_per_device() {
        let result = execute_block_parallel(
            &["good".to_string(), "bad".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |id| async move {
                if id == "bad" {
                    Err(anyhow::anyhow!("bad device error"))
                } else {
                    Ok(vec!["a warning".to_string()])
                }
            },
        )
        .await;

        let good = result
            .device_results
            .iter()
            .find(|r| r.device_id == "good")
            .expect("should find good");
        assert!(good.success, "good device SHALL succeed");
        assert_eq!(good.warnings, vec!["a warning".to_string()]);
        assert!(good.error.is_none(), "successful device SHALL have no error");

        let bad = result
            .device_results
            .iter()
            .find(|r| r.device_id == "bad")
            .expect("should find bad");
        assert!(!bad.success, "bad device SHALL fail");
        assert!(bad.warnings.is_empty());
        assert!(
            bad.error
                .as_ref()
                .is_some_and(|e| e.contains("bad device error")),
            "failed device SHALL retain its error message"
        );
    }

    // ---------------------------------------------------------------
    // 15. max_concurrency larger than device count runs all devices
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn max_concurrency_exceeding_device_count_runs_all() {
        // 1. Over-provision the semaphore (100 permits, 4 devices) so it never
        //    blocks; observe peak concurrency to prove every device runs at
        //    once, unlike the semaphore-bounded case in test 5.
        let device_count = 4;
        let devices: Vec<String> = (0..device_count).map(|i| format!("device{i}")).collect();
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let concurrent_clone = concurrent.clone();
        let peak_clone = peak.clone();

        let result = execute_block_parallel(&devices, 100, move |_id| {
            let concurrent = concurrent_clone.clone();
            let peak = peak_clone.clone();
            async move {
                let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                loop {
                    let old_peak = peak.load(Ordering::SeqCst);
                    if current <= old_peak {
                        break;
                    }
                    match peak.compare_exchange_weak(
                        old_peak,
                        current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                }
                // 2. Hold so all over-provisioned tasks overlap before any exits.
                tokio::time::sleep(Duration::from_millis(50)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
                Ok(Vec::new())
            }
        })
        .await;

        // 3. All devices ran and succeeded.
        assert_eq!(
            result.device_results.len(),
            device_count,
            "all devices SHALL run when concurrency exceeds device count"
        );
        assert!(result.all_succeeded());

        // 4. With permits >> devices, peak concurrency SHALL reach the full
        //    device count (no semaphore throttling), distinguishing this from
        //    the bounded case where peak is capped below device count.
        assert_eq!(
            peak.load(Ordering::SeqCst),
            device_count,
            "over-provisioned semaphore SHALL let every device run concurrently"
        );
    }

    // ---------------------------------------------------------------
    // 16. Multiple warnings from one device are all retained in order
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn multiple_warnings_retained_in_order() {
        let result = execute_block_parallel(
            &["multi".to_string()],
            DEFAULT_MAX_CONCURRENCY,
            |_id| async {
                Ok(vec![
                    "first".to_string(),
                    "second".to_string(),
                    "third".to_string(),
                ])
            },
        )
        .await;

        assert_eq!(result.device_results.len(), 1);
        assert_eq!(
            result.device_results[0].warnings,
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ],
            "warnings SHALL be retained in the order produced"
        );
    }
}
