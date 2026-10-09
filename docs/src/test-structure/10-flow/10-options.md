### Flow Options

```toml
[flow.options]
step_timeout = 5000                 # Base timeout (ms), default: 5000. See Timeout Multipliers.
max_steps = 10000                   # Fail the flow after this many steps (default: 10000)
max_runtime = "30m"                 # Fail the flow after this long (default: "1h"). "5m", "2h", "500ms"
max_device_wait = "30m"             # Fail if no device frees up within this time (default: wait forever). --max-device-wait overrides.
app_lifecycle = "reset"             # "reset" (default), "launch", "manual" — see Lifecycle
screenshot_on_failure = true        # Auto-capture screenshot on step failure (default: true)
record = true                       # Default every block to record (block can opt out with `record = false`)
coverage = "smart"                  # "smart" (default), "min", "full", "one" — see Coverage Strategies
create_if_missing = false           # Create a simulator/emulator when none matches (default: false) — see Hardware Axis
perf = true                         # Performance monitoring (default: true)
a11y = "relaxed"                    # Accessibility audit: "off", "critical", "relaxed" (default), "strict". --a11y overrides.
```

Thresholds are unset by default; the values below are examples. A `*_warn*` threshold adds a warning; a `*_error*` threshold fails the flow.

```toml
[flow.options]
perf_memory_warn_mb = 200.0         # App memory, MB
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0        # App CPU, %
perf_cpu_error_percent = 95.0
perf_threads_warn = 100             # Thread count
perf_threads_error = 200
perf_fd_warn = 200                  # Open file descriptors
perf_fd_error = 500
a11y_max_errors = 0                 # Fail the flow if cumulative a11y errors exceed this
a11y_max_warnings = 20              # Fail the flow if cumulative a11y warnings exceed this
a11y_min_confidence = 0.8           # Drop a11y findings below this confidence (0–1). Deterministic checks are 1.0.
```
