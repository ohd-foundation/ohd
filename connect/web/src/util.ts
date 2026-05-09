// Misc helpers shared across pages.

/** Format a Unix-ms timestamp as `YYYY-MM-DD HH:mm` in local time. */
export function fmtDate(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Relative time: "3m ago", "2h ago", "yesterday", or fall back to fmtDate. */
export function fmtRelative(ms: number, now = Date.now()): string {
  const dt = now - ms;
  if (dt < 0) return fmtDate(ms);
  const sec = Math.round(dt / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  if (day < 7) return `${day}d ago`;
  return fmtDate(ms);
}

/** Pretty-print an ISO 8601 duration `PT5M` etc., or null when unknown. */
export function fmtDays(days: number | null | undefined): string {
  if (days == null) return "indefinite";
  if (days < 1) return `${Math.round(days * 24)}h`;
  if (days < 30) return `${days}d`;
  if (days < 365) return `${Math.round(days / 30)}mo`;
  return `${(days / 365).toFixed(1)}y`;
}

/** Render std.* event type as a short label. */
export function prettyEventType(t: string): string {
  switch (t) {
    case "std.blood_glucose":
      return "Glucose";
    case "std.heart_rate_resting":
      return "Heart rate";
    case "std.body_temperature":
      return "Temperature";
    case "std.blood_pressure":
      return "Blood pressure";
    case "std.medication_dose":
      return "Medication";
    case "std.symptom":
      return "Symptom";
    case "std.meal":
      return "Meal";
    case "std.mood":
      return "Mood";
    case "std.clinical_note":
      return "Clinical note";
    default:
      return t.replace(/^std\./, "");
  }
}
