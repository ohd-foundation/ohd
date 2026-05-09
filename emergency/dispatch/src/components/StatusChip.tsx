import type { CaseStatus } from "../types";

export function StatusChip({ status }: { status: CaseStatus | string }) {
  const cls =
    status === "open"
      ? "chip chip-open"
      : status === "handoff"
        ? "chip chip-handoff"
        : status === "closed"
          ? "chip chip-closed"
          : "chip";
  return <span className={cls}>{status}</span>;
}

export function ResultChip({ result }: { result: string }) {
  const cls =
    result === "success"
      ? "chip chip-success"
      : result === "rejected" || result === "error"
        ? "chip chip-error"
        : result === "partial"
          ? "chip chip-warn"
          : "chip";
  return <span className={cls}>{result}</span>;
}
