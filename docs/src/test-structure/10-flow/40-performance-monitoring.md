### Performance Monitoring

Golem captures app performance metrics after each block (unless `--no-perf` or `perf = false`). Metrics are collected from the device via platform tools and the companion app.

| Metric | Unit | Source |
|--------|------|--------|
| Memory | MB | `dumpsys meminfo` (Android), `footprint_in_bytes` (iOS) |
| CPU | % | `dumpsys cpuinfo` (Android), `cpu_usage` (iOS) |
| Threads | count | `/proc/<pid>/status` (Android), `threadCount` (iOS) |
| File descriptors | count | Companion `/perf` endpoint |
| Disk | MB | Companion `/perf` (Android), `du -sk` (iOS) |
| Network RX/TX | KB | Companion `/perf` (Android), `netstat` (iOS) |

Thresholds in `[flow.options]` trigger warnings or failures:

```toml
perf_memory_warn_mb = 200.0     # Warn if memory exceeds 200 MB
perf_memory_error_mb = 500.0    # Fail if memory exceeds 500 MB
perf_cpu_warn_percent = 80.0    # Warn if CPU exceeds 80%
perf_cpu_error_percent = 95.0   # Fail if CPU exceeds 95%
```

Performance data appears in all output formats: human (table), JSON (objects), JUnit (properties), toon (abbreviated codes).
