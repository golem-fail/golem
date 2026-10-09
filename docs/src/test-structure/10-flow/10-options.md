### Flow Options

```toml
[flow.options]
step_timeout = 5000                 # Base timeout (ms), default: 5000. See timeout multipliers below.
max_steps = 10000                   # Safety limit
max_runtime = "30m"                 # "5m", "2h", "500ms"
app_lifecycle = "reset"             # "reset" (default), "launch", "manual"
screenshot_on_failure = true        # Auto-capture screenshot on step failure (default: true)
record = true                       # Default every block to record (block can opt out with `record = false`)
coverage = "smart"                  # "smart" (default), "min", "full", "one" — see Coverage strategies
perf = true                         # Performance monitoring (default: true)
perf_memory_warn_mb = 200.0
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0
perf_cpu_error_percent = 95.0
a11y = "relaxed"                    # Accessibility audit: "off", "critical", "relaxed" (default), "strict". --a11y overrides.
a11y_max_errors = 0                 # Optional: fail the flow if cumulative a11y errors exceed this
a11y_max_warnings = 20              # Optional: fail the flow if cumulative a11y warnings exceed this
a11y_min_confidence = 0.8           # Optional: drop findings below this confidence (0–1). Deterministic checks are 1.0.
```
