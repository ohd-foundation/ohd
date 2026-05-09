import type { TimelineRow } from "../types";
import { fmtClock, fmtElapsed } from "../util";

interface TimelineFeedProps {
  rows: TimelineRow[];
  /** Reference "now" for the relative ago column. */
  nowMs?: number;
}

/**
 * Vertical event feed used in the case-detail drawer. Compact, mono
 * timestamps on the left, event-type chip in the middle, summary on the
 * right.
 */
export function TimelineFeed({ rows, nowMs = Date.now() }: TimelineFeedProps) {
  if (rows.length === 0) {
    return <div className="empty">No events recorded yet.</div>;
  }
  return (
    <ol className="timeline">
      {rows.map((r, i) => (
        <li className="timeline-item" key={`${r.ts_ms}-${i}`}>
          <div className="timeline-stamp">
            <span className="mono">{fmtClock(r.ts_ms)}</span>
            <span className="muted timeline-ago">{fmtElapsed(nowMs - r.ts_ms)} ago</span>
          </div>
          <div className="timeline-type">{r.event_type}</div>
          <div className="timeline-summary">
            <div>{r.summary}</div>
            {r.detail && <div className="timeline-detail muted">{r.detail}</div>}
          </div>
        </li>
      ))}
    </ol>
  );
}
