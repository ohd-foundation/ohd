import type { ReactNode } from "react";

export function Spinner({ label = "Loading" }: { label?: string }) {
  return <div className="spinner-screen">{label}…</div>;
}

export function ErrorBanner({ message }: { message: string }) {
  return <div className="banner error">{message}</div>;
}

export function InfoBanner({ children }: { children: ReactNode }) {
  return <div className="banner info">{children}</div>;
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="empty">{children}</div>;
}

// Maps a source status string to a coloured pill.
export function StatusPill({ status }: { status: string }) {
  const s = status.toLowerCase();
  let cls = "pill";
  if (s === "ok" || s === "reachable" || s === "connected") cls = "pill ok";
  else if (s === "error" || s === "unreachable" || s === "failed")
    cls = "pill bad";
  else if (s === "pending" || s === "stale" || s === "unknown")
    cls = "pill warn";
  return <span className={cls}>{status}</span>;
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
