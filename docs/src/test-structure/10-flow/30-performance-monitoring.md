### Performance Monitoring

Golem captures app performance metrics after each block (unless `--no-perf` or `perf = false`).

| Metric | Unit |
|--------|------|
| Memory | MB |
| CPU | % |
| Threads | count |
| File descriptors | count |
| Disk | MB |
| Network RX/TX | KB |

The `perf_*` thresholds in [Flow Options](#flow-options) act on memory, CPU, threads and file descriptors: crossing a warn threshold adds a warning, crossing an error threshold fails the flow.

Performance data appears in all output formats: human (table), JSON (objects), JUnit (properties), toon (abbreviated codes).
