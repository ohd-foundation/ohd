// Small UI helpers. Keep this dependency-free.

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/**
 * Render a 16-byte ULID as Crockford-base32 (26 chars). Mirrors the Rust
 * core's `ulid::to_crockford` so wire ULIDs and audit-log strings are
 * comparable byte-for-byte.
 */
export function ulidToCrockford(bytes: Uint8Array | undefined): string {
  if (!bytes || bytes.length !== 16) return "";
  let out = "";
  out += CROCKFORD[(bytes[0] >> 5) & 0x07];
  let buf = bytes[0] & 0x1f;
  let bits = 5;
  for (let i = 1; i < 16; i++) {
    buf = (buf << 8) | bytes[i];
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += CROCKFORD[(buf >> bits) & 0x1f];
    }
  }
  if (bits > 0) {
    out += CROCKFORD[(buf << (5 - bits)) & 0x1f];
  }
  return out;
}

/** Truncate a ULID for table display (keeps the timestamp prefix). */
export function shortUlid(ulid: string): string {
  if (ulid.length <= 10) return ulid;
  return `${ulid.slice(0, 6)}…${ulid.slice(-4)}`;
}

/** "12:04:33" UTC clock (HH:MM:SS, no date). */
export function fmtClock(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  const ss = String(d.getUTCSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss}Z`;
}

/** "2026-05-09 12:04:33Z" full UTC stamp. */
export function fmtStamp(ms: number): string {
  const d = new Date(ms);
  const yyyy = d.getUTCFullYear();
  const mo = String(d.getUTCMonth() + 1).padStart(2, "0");
  const dd = String(d.getUTCDate()).padStart(2, "0");
  return `${yyyy}-${mo}-${dd} ${fmtClock(ms)}`;
}

/** "12m 03s" for elapsed duration ms. */
export function fmtElapsed(ms: number): string {
  if (ms < 0) ms = 0;
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, "0")}s`;
  return `${sec}s`;
}

/** "since 12:04:33Z (3m 12s)" composite stamp. */
export function fmtSinceWithElapsed(ms: number, nowMs: number): string {
  return `${fmtClock(ms)} · ${fmtElapsed(nowMs - ms)} ago`;
}

/** CSV-escape a single field. RFC 4180-ish. */
export function csvEscape(v: unknown): string {
  const s = v == null ? "" : String(v);
  if (s.includes(",") || s.includes("\n") || s.includes('"')) {
    return `"${s.replace(/"/g, '""')}"`;
  }
  return s;
}

/** Build a CSV blob from a header + rows of strings. */
export function toCsv(header: string[], rows: string[][]): string {
  const lines = [header.map(csvEscape).join(",")];
  for (const row of rows) lines.push(row.map(csvEscape).join(","));
  return `${lines.join("\n")}\n`;
}

/** Trigger a download of `text` as `filename`. */
export function downloadText(filename: string, text: string, mime = "text/csv"): void {
  if (typeof window === "undefined") return;
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
