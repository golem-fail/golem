// In-memory event log for debugging tap misses and other input issues.
//
// A document-level pointer listener (capture phase, passive) records every
// pointer-down and pointer-up the WebView actually receives — what the
// app sees, not what golem intended. Pairs with `golem tree` to read the
// log back during a stuck flow: see which element captured the event,
// where it landed inside that element, and how the down/up timings line
// up.
//
// Capture phase + `passive: true` means we observe before app handlers
// run, never preventDefault, and never block. Zero impact on app
// behaviour. Lives in memory only — clears on app restart.

class EventLog {
  entries = $state([]);
  static MAX = 50;

  push(entry) {
    this.entries.unshift(entry);
    if (this.entries.length > EventLog.MAX) {
      this.entries.length = EventLog.MAX;
    }
  }

  clear() {
    this.entries = [];
  }
}

export const eventLog = new EventLog();

function formatTs() {
  const d = new Date();
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${ms}`;
}

function describeTarget(el) {
  if (!el || !(el instanceof Element)) return "?";
  const tag = el.tagName.toLowerCase();
  const aria = el.getAttribute("aria-label");
  if (aria) return `${tag}[${aria}]`;
  const text = (el.textContent || "").trim().slice(0, 30);
  return text ? `${tag}[${text}]` : tag;
}

function logPointer(type, e) {
  let rel = "?";
  if (e.target instanceof Element) {
    const r = e.target.getBoundingClientRect();
    rel = `(${Math.round(e.clientX - r.left)},${Math.round(e.clientY - r.top)})`;
  }
  eventLog.push({
    ts: formatTs(),
    type,
    abs: `(${Math.round(e.clientX)},${Math.round(e.clientY)})`,
    target: describeTarget(e.target),
    rel,
  });
}

// Self-install at import time so events are captured before any
// component mounts the log pane. Guarded for SSR.
if (typeof document !== "undefined") {
  document.addEventListener("pointerdown", (e) => logPointer("down", e), {
    capture: true,
    passive: true,
  });
  document.addEventListener("pointerup", (e) => logPointer("up", e), {
    capture: true,
    passive: true,
  });
}
