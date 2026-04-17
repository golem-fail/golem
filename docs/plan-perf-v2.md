# Perf V2: Multi-App Support + Android Companion Metrics

## Context

Perf capture (shipped in v1) works end-to-end on iOS and Android, but has two gaps:

1. **Multi-app**: `PerfCollector` is created with a single `bundle_id` (the first app). In multi-app flows (launch A, switch to B, back to A), every snapshot only measures app A — even when interacting with app B.

2. **Android gaps**: Three metrics fail due to permissions on non-rooted devices:
   - **FDs**: `ls /proc/{pid}/fd` → permission denied
   - **Disk**: `du /data/data/{pkg}` → permission denied  
   - **Network**: `xt_qtaguid` removed on Android 10+, no host-side replacement

   The Android companion runs as instrumentation (same UID context) and can fill all three gaps via Java APIs.

## Current state (what works)

| Metric | iOS (host) | Android (host) |
|--------|-----------|---------------|
| Memory | `ps -o rss=` | `dumpsys meminfo` |
| CPU | `ps -o %cpu=` | `dumpsys cpuinfo` |
| Threads | `ps -M` | `/proc/{pid}/status` |
| FDs | `lsof -p` | **permission denied** |
| Disk | `simctl get_app_container` + `du` | **permission denied** |
| Network | `nettop -p` | **removed on modern Android** |

iOS stays as-is (host-side). Android gets a companion endpoint.

## Design

### Part 1: Android companion `/perf` endpoint

Add a single `GET /perf?package=<bundle_id>` endpoint to `CompanionServer.java` that returns:

```json
{
  "file_descriptors": 87,
  "disk_kb": 24680,
  "net_rx_bytes": 159744,
  "net_tx_bytes": 46080
}
```

**Implementation in Java:**
- **FDs**: `new File("/proc/" + pid + "/fd").listFiles().length` — companion shares UID, has access
- **Disk**: `executeShellCommand("run-as " + package + " du -sk /data/data/" + package)` — `run-as` executes as the app's UID, which has access to its own data dir. Works on all Android versions, no special permissions.
- **Network**: `TrafficStats.getUidRxBytes(uid)` / `TrafficStats.getUidTxBytes(uid)` — cumulative bytes, always available
- **PID lookup**: `executeShellCommand("pidof <package>")` — already used pattern in companion

All fields are nullable — if any fails, return `null` for that field.

### Part 2: Rust-side changes to consume companion endpoint

**`golem-runner/src/perf.rs`:**
- Add `companion_port: Option<u16>` to `PerfCollector`
- In `capture_android()`: after host-side collection (memory/CPU/threads still via adb), call `GET http://localhost:{port}/perf?package={bundle_id}` to fill FDs, disk, and network
- Remove `xt_qtaguid` parsing code (dead on modern Android)
- Parse companion JSON response, merge into `RawPerfData`

**`golem-cli/src/suite.rs`:**
- Pass the companion port to `PerfCollector::new()` (already available as `port` in `run_flow_on_device`)

### Part 3: Multi-app perf capture

Currently `PerfCollector` is constructed once with `flow.flow.apps.first().bundle`. For multi-app support:

**Option: One collector per app**

Create a `HashMap<String, PerfCollector>` — one entry per app in `[[flow.apps]]`. At block-end capture, determine which app is active (from the block's `app` field, or the last `launch` action's target), and capture from that collector.

**Changes needed:**

1. **`golem-runner/src/perf.rs`**:
   - `PerfCollector::new()` takes the same args (it's already per-bundle)
   - Add `PerfCollectorSet` that holds `HashMap<String, PerfCollector>` + tracks active app
   - `fn set_active_app(&mut self, bundle_id: &str)` — called on `launch`/`stop` actions
   - `fn capture(&self) -> RawPerfData` — delegates to the active app's collector

2. **`golem-runner/src/context.rs`**:
   - Change `perf_collector: Option<&'a PerfCollector>` → `perf_collector: Option<&'a PerfCollectorSet>`

3. **`golem-runner/src/actions/app_lifecycle.rs`**:
   - On `handle_launch`: set active app on the collector set
   - On `handle_stop`: clear active app (or set to previous)

4. **`golem-cli/src/suite.rs`**:
   - Create one `PerfCollector` per app in `[[flow.apps]]`, wrap in `PerfCollectorSet`

5. **Snapshot label**:
   - Include app name: `login:myapp:iPhone_16:0`
   - For unnamed apps, use bundle ID

## Files to modify

| File | Change |
|------|--------|
| `companions/android/.../CompanionServer.java` | Add `/perf` endpoint with FD/disk/network collection |
| `golem-runner/src/perf.rs` | Add `PerfCollectorSet`, companion HTTP call for Android, remove `xt_qtaguid` |
| `golem-runner/src/context.rs` | Change type from `PerfCollector` to `PerfCollectorSet` |
| `golem-runner/src/actions/app_lifecycle.rs` | Track active app on launch/stop |
| `golem-runner/src/executor.rs` | Use `PerfCollectorSet` instead of `PerfCollector` |
| `golem-cli/src/suite.rs` | Create per-app collectors, pass companion port |

## Implementation order

### Phase 1: Android companion `/perf` endpoint
1. Add `handlePerf()` to CompanionServer.java
2. Register `/perf` in the switch statement
3. Implement FD count, disk size, network bytes collection
4. Test manually: `curl http://localhost:8250/perf?package=fail.golem.test`

### Phase 2: Rust consumes companion endpoint
1. Add `companion_port` to `PerfCollector`
2. In `capture_android()`, HTTP GET to companion for FDs/disk/network
3. Remove `xt_qtaguid` code
4. Pass port through from suite.rs
5. Unit tests for companion JSON parsing

### Phase 3: Multi-app support
1. Add `PerfCollectorSet` with active-app tracking
2. Wire into context, executor, actions
3. Update snapshot labeling to include app name
4. Tests for multi-app scenarios

## Verification

1. Run `e2e/cross/tap.test.toml` on Android — verify FDs, disk, and network now appear in perf output
2. Run `e2e/cross/multi_app.test.toml` — verify snapshots capture the correct app's metrics per block
3. Run on iOS — verify nothing changed (still host-side)
4. `cargo test` — all existing + new tests pass
