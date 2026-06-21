use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use golem_devices::Platform;

/// Raw performance data collected from a single capture.
#[derive(Debug, Clone, Default)]
pub struct RawPerfData {
    pub memory_mb: Option<f64>,
    pub cpu_percent: Option<f64>,
    pub threads: Option<u32>,
    pub file_descriptors: Option<u32>,
    pub disk_mb: Option<f64>,
    pub net_rx_kb: Option<f64>,
    pub net_tx_kb: Option<f64>,
}

/// Collects performance metrics from a running app via host-side commands
/// and (on Android) the companion HTTP server.
pub struct PerfCollector {
    platform: Platform,
    device_id: String,
    bundle_id: String,
    /// Companion HTTP port (localhost). Used on Android for FDs, disk, and network.
    companion_port: Option<u16>,
    /// Test-only: when set, `capture` returns this verbatim and skips all device I/O.
    #[cfg(test)]
    forced_raw: Option<RawPerfData>,
}

impl PerfCollector {
    pub fn new(
        platform: Platform,
        device_id: String,
        bundle_id: String,
        companion_port: Option<u16>,
    ) -> Self {
        Self {
            platform,
            device_id,
            bundle_id,
            companion_port,
            #[cfg(test)]
            forced_raw: None,
        }
    }

    /// Test-only constructor that yields the caller-supplied [`RawPerfData`] on
    /// `capture`, bypassing all real adb/companion device I/O. Lets perf and
    /// lifecycle tests exercise the capture pipeline deterministically.
    #[cfg(test)]
    pub(crate) fn from_raw(bundle_id: String, raw: RawPerfData) -> Self {
        Self {
            platform: Platform::Android,
            device_id: "test-device".to_string(),
            bundle_id,
            companion_port: None,
            forced_raw: Some(raw),
        }
    }

    /// Capture a performance snapshot. Returns partial data on failure — never errors.
    pub async fn capture(&self) -> RawPerfData {
        #[cfg(test)]
        if let Some(raw) = &self.forced_raw {
            return raw.clone();
        }
        match self.platform {
            Platform::Android => self.capture_android().await,
            Platform::Ios => self.capture_ios().await,
        }
    }

    async fn capture_android(&self) -> RawPerfData {
        let mut data = RawPerfData::default();

        // Memory: dumpsys meminfo (host-side, works without permissions)
        if let Ok(output) = self
            .adb(&["shell", "dumpsys", "meminfo", &self.bundle_id])
            .await
        {
            data.memory_mb = parse_android_memory(&output);
        }

        // CPU: dumpsys cpuinfo (host-side, works without permissions)
        if let Ok(output) = self.adb(&["shell", "dumpsys", "cpuinfo"]).await {
            data.cpu_percent = parse_android_cpu(&output, &self.bundle_id);
        }

        // Threads: /proc/{pid}/status (host-side, readable by shell user)
        if let Some(pid) = self.android_pid().await {
            if let Ok(output) = self
                .adb(&["shell", "cat", &format!("/proc/{pid}/status")])
                .await
            {
                data.threads = parse_android_threads(&output);
            }
        }

        // FDs, disk, network: companion endpoint (needs app UID / run-as)
        if let Some(port) = self.companion_port {
            if let Ok(perf) = fetch_companion_perf(port, &self.bundle_id).await {
                data.file_descriptors = perf.file_descriptors;
                data.disk_mb = perf.disk_kb.map(|kb| kb as f64 / 1024.0);
                data.net_rx_kb = perf.net_rx_bytes.map(|b| b as f64 / 1024.0);
                data.net_tx_kb = perf.net_tx_bytes.map(|b| b as f64 / 1024.0);
            }
        }

        data
    }

    async fn capture_ios(&self) -> RawPerfData {
        let mut data = RawPerfData::default();

        if let Some(pid) = self.ios_pid().await {
            let pid_str = pid.to_string();

            // Memory: ps -o rss=
            if let Ok(output) = run_cmd("ps", &["-o", "rss=", "-p", &pid_str]).await {
                data.memory_mb = parse_ios_memory(&output);
            }

            // CPU: ps -o %cpu=
            if let Ok(output) = run_cmd("ps", &["-o", "%cpu=", "-p", &pid_str]).await {
                data.cpu_percent = parse_ios_cpu(&output);
            }

            // Threads: ps -M
            if let Ok(output) = run_cmd("ps", &["-M", "-p", &pid_str]).await {
                data.threads = parse_ios_threads(&output);
            }

            // File descriptors: lsof -p
            if let Ok(output) = run_cmd("lsof", &["-p", &pid_str]).await {
                data.file_descriptors = parse_ios_fds(&output);
            }

            // Network: nettop
            if let Ok(output) = run_cmd(
                "nettop",
                &["-p", &pid_str, "-L", "1", "-P", "-k", "bytes_in,bytes_out"],
            )
            .await
            {
                let (rx, tx) = parse_ios_network(&output);
                data.net_rx_kb = rx;
                data.net_tx_kb = tx;
            }
        }

        // Disk: simctl get_app_container + du
        if let Ok(container) = run_cmd(
            "xcrun",
            &[
                "simctl",
                "get_app_container",
                &self.device_id,
                &self.bundle_id,
                "data",
            ],
        )
        .await
        {
            let path = container.trim();
            if !path.is_empty() {
                if let Ok(output) = run_cmd("du", &["-sk", path]).await {
                    data.disk_mb = parse_ios_disk(&output);
                }
            }
        }

        data
    }

    async fn android_pid(&self) -> Option<u32> {
        let output = self.adb(&["shell", "pidof", &self.bundle_id]).await.ok()?;
        output.split_whitespace().next()?.parse().ok()
    }

    async fn ios_pid(&self) -> Option<u32> {
        // Use simctl spawn + launchctl list to find the real app PID.
        // This avoids false matches from `pgrep -f` which picks up log stream
        // and npm processes that have the bundle ID in their arguments.
        let output = run_cmd(
            "xcrun",
            &["simctl", "spawn", &self.device_id, "launchctl", "list"],
        )
        .await
        .ok()?;
        parse_ios_launchctl_pid(&output, &self.bundle_id)
    }

    async fn adb(&self, args: &[&str]) -> Result<String> {
        let output = tokio::process::Command::new("adb")
            .arg("-s")
            .arg(&self.device_id)
            .args(args)
            .output()
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Multi-app performance collector. Holds one `PerfCollector` per app and
/// tracks which app is currently active (foregrounded).
pub struct PerfCollectorSet {
    collectors: HashMap<String, PerfCollector>,
    /// The bundle ID of the currently active (foregrounded) app.
    active_bundle: Mutex<Option<String>>,
}

impl PerfCollectorSet {
    /// Create a collector set for all apps in the flow.
    /// `apps` is a list of `(friendly_name, bundle_id)` pairs.
    pub fn new(
        apps: &[(String, String)],
        platform: Platform,
        device_id: String,
        companion_port: Option<u16>,
    ) -> Self {
        let mut collectors = HashMap::new();
        let mut first_bundle = None;
        for (_, bundle_id) in apps {
            if first_bundle.is_none() {
                first_bundle = Some(bundle_id.clone());
            }
            collectors.insert(
                bundle_id.clone(),
                PerfCollector::new(
                    platform,
                    device_id.clone(),
                    bundle_id.clone(),
                    companion_port,
                ),
            );
        }
        Self {
            collectors,
            active_bundle: Mutex::new(first_bundle),
        }
    }

    /// Test-only constructor: build a set whose collectors yield the supplied
    /// raw data per bundle, with no real device I/O. The first entry is marked
    /// active. Pairs with [`PerfCollector::from_raw`] for lifecycle/perf tests.
    #[cfg(test)]
    pub(crate) fn from_raw(apps: &[(String, RawPerfData)]) -> Self {
        let mut collectors = HashMap::new();
        let mut first_bundle = None;
        for (bundle_id, raw) in apps {
            if first_bundle.is_none() {
                first_bundle = Some(bundle_id.clone());
            }
            collectors.insert(
                bundle_id.clone(),
                PerfCollector::from_raw(bundle_id.clone(), raw.clone()),
            );
        }
        Self {
            collectors,
            active_bundle: Mutex::new(first_bundle),
        }
    }

    /// Set the active app (called on `launch` actions).
    pub fn set_active(&self, bundle_id: &str) {
        if let Ok(mut active) = self.active_bundle.lock() {
            *active = Some(bundle_id.to_string());
        }
    }

    /// Clear the active app (called on `stop` actions).
    pub fn clear_active(&self, bundle_id: &str) {
        if let Ok(mut active) = self.active_bundle.lock() {
            if active.as_deref() == Some(bundle_id) {
                *active = None;
            }
        }
    }

    /// Get the active bundle ID.
    pub fn active_bundle_id(&self) -> Option<String> {
        self.active_bundle.lock().ok()?.clone()
    }

    /// Capture perf for the currently active app.
    pub async fn capture(&self) -> RawPerfData {
        let bundle_id = match self.active_bundle_id() {
            Some(id) => id,
            None => return RawPerfData::default(),
        };
        match self.collectors.get(&bundle_id) {
            Some(collector) => collector.capture().await,
            None => RawPerfData::default(),
        }
    }
}

async fn run_cmd(cmd: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ── Android companion client ─────────────────────────────────────────

/// Response from the companion's `/perf` endpoint.
#[derive(Debug)]
struct CompanionPerfResponse {
    file_descriptors: Option<u32>,
    disk_kb: Option<u64>,
    net_rx_bytes: Option<u64>,
    net_tx_bytes: Option<u64>,
}

/// Fetch FDs, disk, and network from the Android companion's `/perf` endpoint.
async fn fetch_companion_perf(port: u16, package: &str) -> Result<CompanionPerfResponse> {
    let url = format!("http://localhost:{port}/perf?package={package}");
    let body = reqwest::get(&url).await?.text().await?;
    parse_companion_perf_json(&body)
}

fn parse_companion_perf_json(json: &str) -> Result<CompanionPerfResponse> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    Ok(CompanionPerfResponse {
        file_descriptors: v["file_descriptors"].as_u64().map(|n| n as u32),
        disk_kb: v["disk_kb"].as_u64(),
        net_rx_bytes: v["net_rx_bytes"].as_u64(),
        net_tx_bytes: v["net_tx_bytes"].as_u64(),
    })
}

// ── Android parsers ──────────────────────────────────────────────────

/// Parse TOTAL PSS from `dumpsys meminfo <package>` output.
fn parse_android_memory(output: &str) -> Option<f64> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("TOTAL PSS:") || trimmed.starts_with("TOTAL:") {
            // "TOTAL PSS:   142536" or "TOTAL:   142536   ..."
            let after_colon = trimmed.split(':').nth(1)?;
            let kb: f64 = after_colon.split_whitespace().next()?.parse().ok()?;
            return Some(kb / 1024.0);
        }
    }
    None
}

/// Parse CPU percentage for a package from `dumpsys cpuinfo`.
fn parse_android_cpu(output: &str, package: &str) -> Option<f64> {
    // Lines look like: "  23.1% 1234/com.example.app: 15% user + 8.1% kernel"
    // or: " +0% 13819/fail.golem.test: 0% user + 0% kernel"
    for line in output.lines() {
        if line.contains(package) {
            let trimmed = line.trim().trim_start_matches('+');
            let pct_str = trimmed.split('%').next()?.trim();
            return pct_str.parse().ok();
        }
    }
    None
}

/// Parse thread count from `/proc/{pid}/status`.
fn parse_android_threads(output: &str) -> Option<u32> {
    for line in output.lines() {
        if line.starts_with("Threads:") {
            return line.split(':').nth(1)?.trim().parse().ok();
        }
    }
    None
}

/// Parse the PID from `launchctl list` output for an iOS simulator app.
///
/// Lines look like: `81329\t0\tUIKitApplication:fail.golem.test[7351][rb-legacy]`
fn parse_ios_launchctl_pid(output: &str, bundle_id: &str) -> Option<u32> {
    let pattern = format!("UIKitApplication:{bundle_id}");
    for line in output.lines() {
        if line.contains(&pattern) {
            // First field is the PID
            return line.trim().split('\t').next()?.parse().ok();
        }
    }
    None
}

// ── iOS parsers ──────────────────────────────────────────────────────

/// Parse memory from `ps -o rss= -p <pid>` (value in KB).
fn parse_ios_memory(output: &str) -> Option<f64> {
    let kb: f64 = output.trim().parse().ok()?;
    Some(kb / 1024.0)
}

/// Parse CPU from `ps -o %cpu= -p <pid>`.
fn parse_ios_cpu(output: &str) -> Option<f64> {
    output.trim().parse().ok()
}

/// Parse thread count from `ps -M -p <pid>` (each line after header is a thread).
fn parse_ios_threads(output: &str) -> Option<u32> {
    let line_count = output.lines().count();
    if line_count <= 1 {
        None
    } else {
        Some((line_count - 1) as u32) // subtract header
    }
}

/// Parse FD count from `lsof -p <pid>` (each line after header is an FD).
fn parse_ios_fds(output: &str) -> Option<u32> {
    let line_count = output.lines().count();
    if line_count <= 1 {
        None
    } else {
        Some((line_count - 1) as u32) // subtract header
    }
}

/// Parse disk usage from `du -sk <path>` (value in KB).
fn parse_ios_disk(output: &str) -> Option<f64> {
    let first_line = output.lines().next()?;
    let kb: f64 = first_line.split_whitespace().next()?.parse().ok()?;
    Some(kb / 1024.0)
}

/// Parse network from `nettop -p <pid> -L 1 -P -k bytes_in,bytes_out`.
fn parse_ios_network(output: &str) -> (Option<f64>, Option<f64>) {
    // nettop output has header line(s) then data lines with comma-separated values
    // Format: time,interface,bytes_in,bytes_out  OR  process.pid,bytes_in,bytes_out
    for line in output.lines().rev() {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() >= 3 {
            // Try parsing the last two fields as bytes_in, bytes_out
            let bytes_in: Option<f64> = fields[fields.len() - 2].trim().parse().ok();
            let bytes_out: Option<f64> = fields[fields.len() - 1].trim().parse().ok();
            if let (Some(rx), Some(tx)) = (bytes_in, bytes_out) {
                return (Some(rx / 1024.0), Some(tx / 1024.0));
            }
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Android memory ───────────────────────────────────────────────

    #[test]
    fn android_memory_total_pss() {
        let output = r#"Applications Memory Usage (in Kilobytes):
Uptime: 123456 Realtime: 123456

** MEMINFO in pid 1234 [com.example.app] **
                   Pss  Private  Private  SwapPss      Rss     Heap     Heap     Heap
                 Total    Dirty    Clean    Dirty    Total     Size    Alloc     Free
                ------   ------   ------   ------   ------   ------   ------   ------
  Native Heap    12345    12300       45        0    14000    20000    15000     5000
  Dalvik Heap     8765     8700       65        0    10000    12000     9000     3000
        TOTAL:  142536
"#;
        let mb = parse_android_memory(output).expect("SHALL parse TOTAL PSS");
        assert!((mb - 139.19).abs() < 0.1);
    }

    #[test]
    fn android_memory_total_pss_with_label() {
        let output = "  TOTAL PSS:   98304   some extra text\n";
        let mb = parse_android_memory(output).expect("SHALL parse TOTAL PSS with label");
        assert!((mb - 96.0).abs() < 0.01);
    }

    #[test]
    fn android_memory_empty() {
        assert!(parse_android_memory("").is_none());
    }

    #[test]
    fn android_memory_no_total_line() {
        let output = "Some random output\nNo total here\n";
        assert!(parse_android_memory(output).is_none());
    }

    // ── Android CPU ──────────────────────────────────────────────────

    #[test]
    fn android_cpu_parses() {
        let output = r#"CPU usage from 12345 to 67890:
  23.1% 1234/com.example.app: 15% user + 8.1% kernel
  12.5% 5678/system_server: 8% user + 4.5% kernel
  5.2% 9012/com.android.systemui: 3% user + 2.2% kernel
"#;
        let cpu = parse_android_cpu(output, "com.example.app").expect("SHALL parse CPU percentage");
        assert!((cpu - 23.1).abs() < 0.01);
    }

    #[test]
    fn android_cpu_plus_prefix() {
        let output = " +0% 13819/fail.golem.test: 0% user + 0% kernel\n";
        let cpu =
            parse_android_cpu(output, "fail.golem.test").expect("SHALL parse CPU with + prefix");
        assert!((cpu - 0.0).abs() < 0.01);
    }

    #[test]
    fn android_cpu_not_found() {
        let output = "  12.5% 5678/system_server: 8% user\n";
        assert!(parse_android_cpu(output, "com.example.app").is_none());
    }

    #[test]
    fn android_cpu_empty() {
        assert!(parse_android_cpu("", "com.example.app").is_none());
    }

    // ── Android threads ──────────────────────────────────────────────

    #[test]
    fn android_threads_parses() {
        let output = r#"Name:   com.example.app
State:  S (sleeping)
Tgid:   1234
Pid:    1234
PPid:   567
Threads:        42
"#;
        assert_eq!(
            parse_android_threads(output).expect("SHALL parse thread count"),
            42
        );
    }

    #[test]
    fn android_threads_empty() {
        assert!(parse_android_threads("").is_none());
    }

    // ── Companion JSON parser ─────────────────────────────────────────

    #[test]
    fn companion_perf_json_full() {
        let json =
            r#"{"file_descriptors":241,"disk_kb":4924,"net_rx_bytes":159744,"net_tx_bytes":46080}"#;
        let r = parse_companion_perf_json(json).expect("SHALL parse companion JSON");
        assert_eq!(r.file_descriptors, Some(241));
        assert_eq!(r.disk_kb, Some(4924));
        assert_eq!(r.net_rx_bytes, Some(159744));
        assert_eq!(r.net_tx_bytes, Some(46080));
    }

    #[test]
    fn companion_perf_json_nulls() {
        let json =
            r#"{"file_descriptors":null,"disk_kb":null,"net_rx_bytes":null,"net_tx_bytes":null}"#;
        let r = parse_companion_perf_json(json).expect("SHALL parse companion JSON with nulls");
        assert!(r.file_descriptors.is_none());
        assert!(r.disk_kb.is_none());
        assert!(r.net_rx_bytes.is_none());
        assert!(r.net_tx_bytes.is_none());
    }

    #[test]
    fn companion_perf_json_partial() {
        let json =
            r#"{"file_descriptors":87,"disk_kb":null,"net_rx_bytes":1024,"net_tx_bytes":null}"#;
        let r = parse_companion_perf_json(json).expect("SHALL parse partial companion JSON");
        assert_eq!(r.file_descriptors, Some(87));
        assert!(r.disk_kb.is_none());
        assert_eq!(r.net_rx_bytes, Some(1024));
        assert!(r.net_tx_bytes.is_none());
    }

    // ── iOS memory ───────────────────────────────────────────────────

    #[test]
    fn ios_memory_parses() {
        let output = "  145920\n";
        let mb = parse_ios_memory(output).expect("SHALL parse RSS");
        assert!((mb - 142.5).abs() < 0.01);
    }

    #[test]
    fn ios_memory_empty() {
        assert!(parse_ios_memory("").is_none());
    }

    #[test]
    fn ios_memory_not_a_number() {
        assert!(parse_ios_memory("not a number\n").is_none());
    }

    // ── iOS CPU ──────────────────────────────────────────────────────

    #[test]
    fn ios_cpu_parses() {
        let output = "  23.1\n";
        let cpu = parse_ios_cpu(output).expect("SHALL parse CPU");
        assert!((cpu - 23.1).abs() < 0.01);
    }

    #[test]
    fn ios_cpu_zero() {
        let output = "0.0\n";
        let cpu = parse_ios_cpu(output).expect("SHALL parse zero CPU");
        assert!((cpu - 0.0).abs() < 0.01);
    }

    #[test]
    fn ios_cpu_empty() {
        assert!(parse_ios_cpu("").is_none());
    }

    // ── iOS threads ──────────────────────────────────────────────────

    #[test]
    fn ios_threads_parses() {
        let output = r#"USER       PID   TT   %CPU STAT PRI     STIME     UTIME COMMAND
user     12345   ??    0.0 S    31T   0:00.01   0:00.03 /path/to/app
user     12345   ??    0.0 S    31T   0:00.00   0:00.01 /path/to/app
user     12345   ??    0.0 S    31T   0:00.02   0:00.05 /path/to/app
"#;
        assert_eq!(parse_ios_threads(output).expect("SHALL count threads"), 3);
    }

    #[test]
    fn ios_threads_header_only() {
        let output = "USER  PID  TT  %CPU STAT PRI STIME UTIME COMMAND\n";
        assert!(parse_ios_threads(output).is_none());
    }

    #[test]
    fn ios_threads_empty() {
        assert!(parse_ios_threads("").is_none());
    }

    // ── iOS FDs ──────────────────────────────────────────────────────

    #[test]
    fn ios_fds_parses() {
        let mut output =
            String::from("COMMAND     PID   USER   FD   TYPE DEVICE SIZE/OFF NODE NAME\n");
        for i in 0..87 {
            output.push_str(&format!(
                "app     12345   user  {i}u   REG  1,4  12345 67890 /some/path\n"
            ));
        }
        assert_eq!(parse_ios_fds(&output).expect("SHALL count FDs"), 87);
    }

    #[test]
    fn ios_fds_header_only() {
        let output = "COMMAND  PID  USER  FD  TYPE  DEVICE  SIZE/OFF  NODE  NAME\n";
        assert!(parse_ios_fds(output).is_none());
    }

    // ── iOS disk ─────────────────────────────────────────────────────

    #[test]
    fn ios_disk_parses() {
        let output = "24680\t/Users/user/Library/Developer/CoreSimulator/...\n";
        let mb = parse_ios_disk(output).expect("SHALL parse disk usage");
        assert!((mb - 24.1).abs() < 0.1);
    }

    #[test]
    fn ios_disk_empty() {
        assert!(parse_ios_disk("").is_none());
    }

    // ── iOS network ──────────────────────────────────────────────────

    #[test]
    fn ios_network_parses() {
        let output = "time,interface,bytes_in,bytes_out\n,en0,159744,46080\n";
        let (rx, tx) = parse_ios_network(output);
        let rx = rx.expect("SHALL parse bytes_in");
        let tx = tx.expect("SHALL parse bytes_out");
        assert!((rx - 156.0).abs() < 0.01);
        assert!((tx - 45.0).abs() < 0.01);
    }

    #[test]
    fn ios_network_empty() {
        let (rx, tx) = parse_ios_network("");
        assert!(rx.is_none());
        assert!(tx.is_none());
    }

    #[test]
    fn ios_network_header_only() {
        let (rx, tx) = parse_ios_network("time,interface,bytes_in,bytes_out\n");
        assert!(rx.is_none());
        assert!(tx.is_none());
    }

    // ── iOS launchctl PID ────────────────────────────────────────────

    #[test]
    fn ios_launchctl_pid_parses() {
        let output = "80823\t0\tUIKitApplication:fail.golem.runner.uitests.xctrunner[90d1][rb-legacy]\n81329\t0\tUIKitApplication:fail.golem.test[7351][rb-legacy]\n";
        let pid = parse_ios_launchctl_pid(output, "fail.golem.test")
            .expect("SHALL parse PID from launchctl list");
        assert_eq!(pid, 81329);
    }

    #[test]
    fn ios_launchctl_pid_not_found() {
        let output = "80823\t0\tUIKitApplication:com.other.app[1234][rb-legacy]\n";
        assert!(parse_ios_launchctl_pid(output, "fail.golem.test").is_none());
    }

    #[test]
    fn ios_launchctl_pid_empty() {
        assert!(parse_ios_launchctl_pid("", "fail.golem.test").is_none());
    }

    // ── Companion JSON error paths ────────────────────────────────────

    // 1. Malformed JSON SHALL surface as an Err, not a panic.
    #[test]
    fn companion_perf_json_invalid() {
        assert!(parse_companion_perf_json("not json at all").is_err());
    }

    // 2. Empty input SHALL be an Err (serde rejects empty document).
    #[test]
    fn companion_perf_json_empty() {
        assert!(parse_companion_perf_json("").is_err());
    }

    // 3. Valid JSON missing the perf keys SHALL parse with all fields None.
    #[test]
    fn companion_perf_json_missing_keys() {
        let r = parse_companion_perf_json("{}").expect("SHALL parse empty object");
        assert!(r.file_descriptors.is_none());
        assert!(r.disk_kb.is_none());
        assert!(r.net_rx_bytes.is_none());
        assert!(r.net_tx_bytes.is_none());
    }

    // 4. Non-numeric values for numeric keys SHALL be treated as absent (None).
    #[test]
    fn companion_perf_json_wrong_types() {
        let json =
            r#"{"file_descriptors":"abc","disk_kb":true,"net_rx_bytes":[],"net_tx_bytes":1.5}"#;
        let r = parse_companion_perf_json(json).expect("SHALL parse JSON with wrong types");
        assert!(r.file_descriptors.is_none());
        assert!(r.disk_kb.is_none());
        assert!(r.net_rx_bytes.is_none());
        assert!(r.net_tx_bytes.is_none());
    }

    // ── Android parser boundary / error paths ─────────────────────────

    // 1. A TOTAL line whose value is non-numeric SHALL yield None.
    #[test]
    fn android_memory_total_non_numeric() {
        assert!(parse_android_memory("        TOTAL:  not_a_number\n").is_none());
    }

    // 2. A TOTAL line with no value after the colon SHALL yield None.
    #[test]
    fn android_memory_total_no_value() {
        assert!(parse_android_memory("        TOTAL:\n").is_none());
    }

    // 3. A line containing the package but an unparseable percentage SHALL yield None.
    #[test]
    fn android_cpu_unparseable_percent() {
        let output = "  abc% 1234/com.example.app: 15% user\n";
        assert!(parse_android_cpu(output, "com.example.app").is_none());
    }

    // 4. A Threads line with a non-numeric value SHALL yield None.
    #[test]
    fn android_threads_non_numeric() {
        assert!(parse_android_threads("Threads:        many\n").is_none());
    }

    // ── iOS parser boundary / error paths ─────────────────────────────

    // 1. A disk line whose first field is non-numeric SHALL yield None.
    #[test]
    fn ios_disk_non_numeric() {
        assert!(parse_ios_disk("xyz\t/some/path\n").is_none());
    }

    // 2. A launchctl line for the matching bundle with a non-numeric PID SHALL yield None.
    #[test]
    fn ios_launchctl_pid_non_numeric() {
        let output = "notapid\t0\tUIKitApplication:fail.golem.test[7351][rb-legacy]\n";
        assert!(parse_ios_launchctl_pid(output, "fail.golem.test").is_none());
    }

    // 3. nettop lines with >=3 fields but unparseable byte counts SHALL yield (None, None).
    #[test]
    fn ios_network_unparseable_bytes() {
        let output = "time,interface,bytes_in,bytes_out\n,en0,foo,bar\n";
        let (rx, tx) = parse_ios_network(output);
        assert!(rx.is_none());
        assert!(tx.is_none());
    }

    // 4. nettop SHALL scan from the bottom and return the last parseable data row.
    #[test]
    fn ios_network_uses_last_data_row() {
        let output = "header,a,b\n,en0,1024,2048\n,en0,4096,8192\n";
        let (rx, tx) = parse_ios_network(output);
        let rx = rx.expect("SHALL parse last row bytes_in");
        let tx = tx.expect("SHALL parse last row bytes_out");
        assert!((rx - 4.0).abs() < 0.01, "last row bytes_in SHALL win");
        assert!((tx - 8.0).abs() < 0.01, "last row bytes_out SHALL win");
    }

    // ── PerfCollectorSet state management ─────────────────────────────

    // 1. new SHALL default the active bundle to the first app in the list.
    #[test]
    fn collector_set_first_app_active() {
        let apps = vec![
            ("Main".to_string(), "com.example.main".to_string()),
            ("Other".to_string(), "com.example.other".to_string()),
        ];
        let set = PerfCollectorSet::new(&apps, Platform::Android, "device-1".to_string(), None);
        assert_eq!(
            set.active_bundle_id().as_deref(),
            Some("com.example.main"),
            "first app SHALL be active by default"
        );
    }

    // 2. An empty app list SHALL leave the active bundle unset.
    #[test]
    fn collector_set_empty_has_no_active() {
        let set = PerfCollectorSet::new(&[], Platform::Ios, "device-1".to_string(), None);
        assert!(
            set.active_bundle_id().is_none(),
            "empty set SHALL have no active bundle"
        );
    }

    // 3. set_active SHALL switch the active bundle.
    #[test]
    fn collector_set_set_active_switches() {
        let apps = vec![
            ("Main".to_string(), "com.example.main".to_string()),
            ("Other".to_string(), "com.example.other".to_string()),
        ];
        let set = PerfCollectorSet::new(&apps, Platform::Android, "device-1".to_string(), None);
        set.set_active("com.example.other");
        assert_eq!(
            set.active_bundle_id().as_deref(),
            Some("com.example.other"),
            "set_active SHALL change the active bundle"
        );
    }

    // 4. clear_active SHALL clear only when the cleared bundle is the active one.
    #[test]
    fn collector_set_clear_active_matching() {
        let apps = vec![("Main".to_string(), "com.example.main".to_string())];
        let set = PerfCollectorSet::new(&apps, Platform::Android, "device-1".to_string(), None);
        set.clear_active("com.example.main");
        assert!(
            set.active_bundle_id().is_none(),
            "clearing the active bundle SHALL unset it"
        );
    }

    // 5. clear_active for a non-active bundle SHALL leave the active bundle intact.
    #[test]
    fn collector_set_clear_active_non_matching() {
        let apps = vec![
            ("Main".to_string(), "com.example.main".to_string()),
            ("Other".to_string(), "com.example.other".to_string()),
        ];
        let set = PerfCollectorSet::new(&apps, Platform::Android, "device-1".to_string(), None);
        set.clear_active("com.example.other");
        assert_eq!(
            set.active_bundle_id().as_deref(),
            Some("com.example.main"),
            "clearing a non-active bundle SHALL not change the active one"
        );
    }

    // 6. capture with no active bundle SHALL return default data without touching a device.
    #[tokio::test]
    async fn collector_set_capture_no_active() {
        let set = PerfCollectorSet::new(&[], Platform::Android, "device-1".to_string(), None);
        let data = set.capture().await;
        assert!(
            data.memory_mb.is_none(),
            "no active bundle SHALL yield empty data"
        );
        assert!(data.cpu_percent.is_none());
        assert!(data.threads.is_none());
    }

    // 7. capture when the active bundle has no registered collector SHALL return default data.
    #[tokio::test]
    async fn collector_set_capture_unknown_bundle() {
        let set = PerfCollectorSet::new(&[], Platform::Android, "device-1".to_string(), None);
        set.set_active("com.example.unregistered");
        let data = set.capture().await;
        assert!(
            data.memory_mb.is_none(),
            "unknown active bundle SHALL yield empty data"
        );
        assert!(data.file_descriptors.is_none());
    }

    // 8. from_raw seam: PerfCollector::from_raw SHALL return the supplied data on
    //    capture, with no device I/O.
    #[tokio::test]
    async fn collector_from_raw_yields_supplied_data() {
        let raw = RawPerfData {
            memory_mb: Some(42.5),
            cpu_percent: Some(7.0),
            threads: Some(9),
            ..RawPerfData::default()
        };
        let collector = PerfCollector::from_raw("com.example.main".to_string(), raw);
        let data = collector.capture().await;
        assert_eq!(
            data.memory_mb,
            Some(42.5),
            "from_raw SHALL return the injected memory verbatim"
        );
        assert_eq!(data.cpu_percent, Some(7.0));
        assert_eq!(data.threads, Some(9));
    }

    // 9. from_raw set seam: PerfCollectorSet::from_raw SHALL capture per active app
    //    from injected data, with no device I/O.
    #[tokio::test]
    async fn collector_set_from_raw_captures_active() {
        let apps = vec![
            (
                "com.example.main".to_string(),
                RawPerfData {
                    memory_mb: Some(100.0),
                    ..RawPerfData::default()
                },
            ),
            (
                "com.example.other".to_string(),
                RawPerfData {
                    memory_mb: Some(200.0),
                    ..RawPerfData::default()
                },
            ),
        ];
        let set = PerfCollectorSet::from_raw(&apps);
        assert_eq!(
            set.capture().await.memory_mb,
            Some(100.0),
            "first app SHALL be active and yield its injected data"
        );
        set.set_active("com.example.other");
        assert_eq!(
            set.capture().await.memory_mb,
            Some(200.0),
            "after switching, capture SHALL yield the new active app's injected data"
        );
    }
}
