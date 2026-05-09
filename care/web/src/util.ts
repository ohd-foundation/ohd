// Small formatting helpers for the OHD Care v0 shell.

export function fmtRelative(ts_ms: number | null | undefined, now: number = Date.now()): string {
  if (ts_ms == null) return "never";
  const diff = now - ts_ms;
  const abs = Math.abs(diff);
  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (abs < minute) return diff >= 0 ? "just now" : "in <1m";
  if (abs < hour) {
    const m = Math.round(abs / minute);
    return diff >= 0 ? `${m}m ago` : `in ${m}m`;
  }
  if (abs < day) {
    const h = Math.round(abs / hour);
    return diff >= 0 ? `${h}h ago` : `in ${h}h`;
  }
  const d = Math.round(abs / day);
  return diff >= 0 ? `${d}d ago` : `in ${d}d`;
}

export function fmtDate(ts_ms: number): string {
  const d = new Date(ts_ms);
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${mi}`;
}

export function fmtTime(ts_ms: number): string {
  const d = new Date(ts_ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function fmtScope(scope: string[]): string {
  if (!scope || scope.length === 0) return "—";
  return scope.join(", ");
}

export function fmtApprovalMode(mode: string): string {
  switch (mode) {
    case "always":
      return "every write queues for patient approval";
    case "auto_for_event_types":
      return "auto-commit selected types; rest queue for approval";
    case "never_required":
      return "auto-commit all (trusted / break-glass)";
    default:
      return mode;
  }
}
