### Accessibility Audit

After each block, Golem audits the **visible** UI tree for accessibility issues
(zero config, on by default at `relaxed`). Findings appear inline in the live run
and in every report format; they're warnings by default — set `a11y_max_errors` /
`a11y_max_warnings` to fail a flow, and `a11y_min_confidence` to filter heuristic
noise. Levels: `off`, `critical` (tree checks only), `relaxed` (default), `strict`
(adds the screenshot contrast check + an annotated screenshot).

Full guide — the checks, per-level thresholds, the confidence model, and how to
read the annotated screenshot — in **[accessibility.md](accessibility.md)**.
