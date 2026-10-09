### Accessibility Audit

After each block, Golem audits the **visible** UI tree for accessibility issues
(zero config, on by default at `relaxed`). Findings appear inline in the live run
and in every report format. They are warnings unless a flow sets the `a11y_*`
thresholds in [Flow Options](#flow-options). Levels: `off`, `critical` (tree
checks only), `relaxed` (default), `strict` (adds the screenshot contrast check +
an annotated screenshot).

Full guide — the checks, per-level thresholds, the confidence model, and how to
read the annotated screenshot — in **[accessibility.md](accessibility.md)**.
