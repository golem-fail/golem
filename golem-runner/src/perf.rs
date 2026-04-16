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

/// Collects performance metrics from a running app via host-side commands.
pub struct PerfCollector {
    platform: Platform,
    device_id: String,
    bundle_id: String,
}

impl PerfCollector {
    pub fn new(platform: Platform, device_id: String, bundle_id: String) -> Self {
        Self {
            platform,
            device_id,
            bundle_id,
        }
    }

    /// Capture a performance snapshot. Returns partial data on failure — never errors.
    pub async fn capture(&self) -> RawPerfData {
        match self.platform {
            Platform::Android => self.capture_android().await,
            Platform::Ios => self.capture_ios().await,
        }
    }

    async fn capture_android(&self) -> RawPerfData {
        let mut data = RawPerfData::default();

        // Memory: dumpsys meminfo
        if let Ok(output) = self.adb(&["shell", "dumpsys", "meminfo", &self.bundle_id]).await {
            data.memory_mb = parse_android_memory(&output);
        }

        // CPU: dumpsys cpuinfo
        if let Ok(output) = self.adb(&["shell", "dumpsys", "cpuinfo"]).await {
            data.cpu_percent = parse_android_cpu(&output, &self.bundle_id);
        }

        // PID-dependent metrics
        if let Some(pid) = self.android_pid().await {
            // Threads
            if let Ok(output) = self
                .adb(&["shell", "cat", &format!("/proc/{pid}/status")])
                .await
            {
                data.threads = parse_android_threads(&output);
            }
            // File descriptors
            if let Ok(output) = self
                .adb(&["shell", "ls", &format!("/proc/{pid}/fd")])
                .await
            {
                data.file_descriptors = parse_android_fds(&output);
            }
        }

        // Disk: du on app data directory
        if let Ok(output) = self
            .adb(&[
                "shell",
                "du",
                "-sk",
                &format!("/data/data/{}", self.bundle_id),
            ])
            .await
        {
            data.disk_mb = parse_android_disk(&output);
        }

        // Network: /proc/net/xt_qtaguid/stats filtered by UID
        if let Ok(uid_output) = self
            .adb(&[
                "shell",
                "dumpsys",
                "package",
                &self.bundle_id,
            ])
            .await
        {
            if let Some(uid) = parse_android_uid(&uid_output) {
                if let Ok(net_output) = self
                    .adb(&["shell", "cat", "/proc/net/xt_qtaguid/stats"])
                    .await
                {
                    let (rx, tx) = parse_android_network(&net_output, uid);
                    data.net_rx_kb = rx;
                    data.net_tx_kb = tx;
                }
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
        let output = self
            .adb(&["shell", "pidof", &self.bundle_id])
            .await
            .ok()?;
        output.trim().split_whitespace().next()?.parse().ok()
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

async fn run_cmd(cmd: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ── Android parsers ──────────────────────────────────────────────────

/// Parse TOTAL PSS from `dumpsys meminfo <package>` output.
fn parse_android_memory(output: &str) -> Option<f64> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("TOTAL PSS:") || trimmed.starts_with("TOTAL:") {
            // "TOTAL PSS:   142536" or "TOTAL:   142536   ..."
            let after_colon = trimmed.split(':').nth(1)?;
            let kb: f64 = after_colon.trim().split_whitespace().next()?.parse().ok()?;
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

/// Parse file descriptor count from `ls /proc/{pid}/fd`.
fn parse_android_fds(output: &str) -> Option<u32> {
    let count = output.lines().filter(|l| !l.trim().is_empty()).count();
    if count == 0 {
        None
    } else {
        Some(count as u32)
    }
}

/// Parse app disk usage from `du -sk /data/data/<package>`.
fn parse_android_disk(output: &str) -> Option<f64> {
    let first_line = output.lines().next()?;
    let kb: f64 = first_line.trim().split_whitespace().next()?.parse().ok()?;
    Some(kb / 1024.0)
}

/// Extract UID from `dumpsys package <package>` output.
fn parse_android_uid(output: &str) -> Option<u32> {
    // Newer Android: "uid=10198 gids=[] type=0 ..."
    // Older Android: "userId=10123"
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("uid=") {
            return rest.split_whitespace().next()?.parse().ok();
        }
        if let Some(rest) = trimmed.strip_prefix("userId=") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Parse network stats from `/proc/net/xt_qtaguid/stats` filtered by UID.
fn parse_android_network(output: &str, uid: u32) -> (Option<f64>, Option<f64>) {
    let uid_str = uid.to_string();
    let mut rx_bytes: u64 = 0;
    let mut tx_bytes: u64 = 0;
    let mut found = false;

    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Format: idx iface acct_tag_hex uid_tag_int cnt_set rx_bytes rx_packets tx_bytes tx_packets
        if fields.len() >= 8 && fields[3] == uid_str {
            if let (Ok(rx), Ok(tx)) = (fields[5].parse::<u64>(), fields[7].parse::<u64>()) {
                rx_bytes += rx;
                tx_bytes += tx;
                found = true;
            }
        }
    }

    if found {
        (
            Some(rx_bytes as f64 / 1024.0),
            Some(tx_bytes as f64 / 1024.0),
        )
    } else {
        (None, None)
    }
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
    let kb: f64 = first_line.trim().split_whitespace().next()?.parse().ok()?;
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
        let cpu =
            parse_android_cpu(output, "com.example.app").expect("SHALL parse CPU percentage");
        assert!((cpu - 23.1).abs() < 0.01);
    }

    #[test]
    fn android_cpu_plus_prefix() {
        let output = " +0% 13819/fail.golem.test: 0% user + 0% kernel\n";
        let cpu = parse_android_cpu(output, "fail.golem.test")
            .expect("SHALL parse CPU with + prefix");
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

    // ── Android FDs ──────────────────────────────────────────────────

    #[test]
    fn android_fds_parses() {
        let output = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        assert_eq!(
            parse_android_fds(output).expect("SHALL parse FD count"),
            10
        );
    }

    #[test]
    fn android_fds_empty() {
        assert!(parse_android_fds("").is_none());
    }

    #[test]
    fn android_fds_blank_lines() {
        let output = "0\n1\n\n2\n";
        assert_eq!(parse_android_fds(output).expect("SHALL skip blank lines"), 3);
    }

    // ── Android disk ─────────────────────────────────────────────────

    #[test]
    fn android_disk_parses() {
        let output = "24680\t/data/data/com.example.app\n";
        let mb = parse_android_disk(output).expect("SHALL parse disk usage");
        assert!((mb - 24.1).abs() < 0.1);
    }

    #[test]
    fn android_disk_empty() {
        assert!(parse_android_disk("").is_none());
    }

    // ── Android UID ──────────────────────────────────────────────────

    #[test]
    fn android_uid_parses() {
        let output = r#"Packages:
  Package [com.example.app] (abc1234):
    userId=10123
    pkg=Package{abc1234 com.example.app}
"#;
        assert_eq!(
            parse_android_uid(output).expect("SHALL parse UID"),
            10123
        );
    }

    #[test]
    fn android_uid_modern_format() {
        let output = "    uid=10198 gids=[] type=0 prot=signature\n    installerPackageUid=-1\n";
        assert_eq!(
            parse_android_uid(output).expect("SHALL parse modern uid= format"),
            10198
        );
    }

    #[test]
    fn android_uid_not_found() {
        assert!(parse_android_uid("no userId here\n").is_none());
    }

    // ── Android network ──────────────────────────────────────────────

    #[test]
    fn android_network_parses() {
        let output = r#"idx iface acct_tag_hex uid_tag_int cnt_set rx_bytes rx_packets tx_bytes tx_packets
2 wlan0 0x0 10123 0 156000 120 32000 80
3 wlan0 0x0 10123 1 4000 10 1000 5
4 wlan0 0x0 10999 0 999999 500 888888 400
"#;
        let (rx, tx) = parse_android_network(output, 10123);
        let rx = rx.expect("SHALL parse rx bytes");
        let tx = tx.expect("SHALL parse tx bytes");
        // (156000 + 4000) / 1024 = 156.25
        assert!((rx - 156.25).abs() < 0.01);
        // (32000 + 1000) / 1024 = 32.22...
        assert!((tx - 32.226).abs() < 0.01);
    }

    #[test]
    fn android_network_no_matching_uid() {
        let output = "idx iface acct_tag_hex uid_tag_int cnt_set rx_bytes rx_packets tx_bytes tx_packets\n2 wlan0 0x0 10999 0 999 10 888 5\n";
        let (rx, tx) = parse_android_network(output, 10123);
        assert!(rx.is_none());
        assert!(tx.is_none());
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
        assert_eq!(
            parse_ios_threads(output).expect("SHALL count threads"),
            3
        );
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
        let mut output = String::from("COMMAND     PID   USER   FD   TYPE DEVICE SIZE/OFF NODE NAME\n");
        for i in 0..87 {
            output.push_str(&format!("app     12345   user  {i}u   REG  1,4  12345 67890 /some/path\n"));
        }
        assert_eq!(
            parse_ios_fds(&output).expect("SHALL count FDs"),
            87
        );
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
}
