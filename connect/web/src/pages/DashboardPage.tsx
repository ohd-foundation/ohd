import { useMemo } from "react";
import { Sparkline } from "../components/Sparkline";
import { fmtRelative, prettyEventType } from "../util";
import { getSnapshot, READ_EVENT_TYPES } from "../ohdc/store";
import { useStoreVersion } from "../ohdc/useStore";
import type { Event } from "../ohdc/client";

/**
 * Dashboard — recent activity feed + per-channel sparklines.
 *
 * Layout:
 *   - Top: at-a-glance stats (event count, last write).
 *   - Mid: sparkline cards for the numeric std.* event types we have data
 *     for. Each shows the last value, unit, and a 60-px-tall SVG line.
 *   - Bottom: most-recent 50 events as a list (ULID, type, summary).
 */
export function DashboardPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const events = snap.events;

  const now = Date.now();
  const last = events[0];
  const sparkData = useMemo(() => buildSparkData(events), [events]);

  return (
    <section data-testid="dashboard-page">
      <header className="page-header">
        <div>
          <h1>Dashboard</h1>
          <p>{events.length} event{events.length === 1 ? "" : "s"} loaded — last write {last ? fmtRelative(Number(last.timestampMs), now) : "never"}.</p>
        </div>
      </header>

      {sparkData.length === 0 ? (
        <div className="empty">
          <p>Nothing yet. Use the <a href="/log">Log</a> tab to record something.</p>
        </div>
      ) : (
        sparkData.map((c) => (
          <div key={c.eventType} className="card">
            <div className="card-title">
              <h3>{prettyEventType(c.eventType)}</h3>
              <span className="muted mono" style={{ fontSize: 12 }}>
                {c.points.length} reading{c.points.length === 1 ? "" : "s"} · last{" "}
                {fmtRelative(c.points[c.points.length - 1].ts, now)}
              </span>
            </div>
            {c.points.length > 1 ? <Sparkline points={c.points.map((p) => p.v)} /> : null}
            <div className="muted mono" style={{ marginTop: 4, fontSize: 12 }}>
              {c.points.length > 0 ? (
                <>
                  current <strong style={{ color: "var(--color-ink)" }}>{c.points[c.points.length - 1].v.toFixed(2)}</strong>
                  {c.unit ? ` ${c.unit}` : ""}
                </>
              ) : null}
            </div>
          </div>
        ))
      )}

      <div className="card">
        <div className="card-title">
          <h3>Recent activity</h3>
          <span className="muted" style={{ fontSize: 12 }}>last {Math.min(events.length, 50)}</span>
        </div>
        {events.length === 0 ? (
          <div className="empty" style={{ padding: 16 }}>
            No events yet.
          </div>
        ) : (
          <ul className="list-rows">
            {events.slice(0, 50).map((e, idx) => (
              <li key={`${idx}-${Number(e.timestampMs)}`} className="list-row">
                <span className="list-row-time">{fmtRelative(Number(e.timestampMs), now)}</span>
                <span>
                  <strong>{prettyEventType(e.eventType)}</strong>
                  <div className="list-row-detail">{summarizeEvent(e)}</div>
                </span>
                <span className="badge badge-info">{e.eventType}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

interface SparkChannel {
  eventType: string;
  unit?: string;
  points: { ts: number; v: number }[];
}

function buildSparkData(events: Event[]): SparkChannel[] {
  // Build a per-event-type series for the numeric event types only.
  const seriesMap = new Map<string, SparkChannel>();
  const MEASUREMENT_TYPES = new Set([
    "std.blood_glucose",
    "std.heart_rate_resting",
    "std.body_temperature",
    "std.blood_pressure",
  ]);
  for (const e of events) {
    if (!MEASUREMENT_TYPES.has(e.eventType)) continue;
    const key = e.eventType;
    let ch = seriesMap.get(key);
    if (!ch) {
      ch = { eventType: key, unit: unitFor(key), points: [] };
      seriesMap.set(key, ch);
    }
    const v = primaryValue(e);
    if (v != null) ch.points.push({ ts: Number(e.timestampMs), v });
  }
  for (const ch of seriesMap.values()) {
    ch.points.sort((a, b) => a.ts - b.ts);
  }
  // Order by READ_EVENT_TYPES so sparklines render in a stable, expected order.
  const order = new Map<string, number>();
  READ_EVENT_TYPES.forEach((t, i) => order.set(t, i));
  return [...seriesMap.values()].sort((a, b) => (order.get(a.eventType) ?? 99) - (order.get(b.eventType) ?? 99));
}

function unitFor(eventType: string): string | undefined {
  switch (eventType) {
    case "std.blood_glucose":
      return "mmol/L";
    case "std.heart_rate_resting":
      return "bpm";
    case "std.body_temperature":
      return "°C";
    case "std.blood_pressure":
      return "mmHg";
    default:
      return undefined;
  }
}

function primaryValue(e: Event): number | null {
  // Prefer "value", then "bpm", then "systolic" — matches how the storage
  // server seeds these channels (`storage/migrations/002_std_registry.sql`).
  for (const path of ["value", "bpm", "systolic"]) {
    for (const c of e.channels) {
      if (c.channelPath === path && c.value.case === "realValue") return c.value.value;
      if (c.channelPath === path && c.value.case === "intValue") return Number(c.value.value);
    }
  }
  return null;
}

function summarizeEvent(e: Event): string {
  const bits: string[] = [];
  for (const c of e.channels) {
    let v = "";
    switch (c.value.case) {
      case "realValue":
        v = c.value.value.toFixed(2);
        break;
      case "intValue":
        v = String(c.value.value);
        break;
      case "boolValue":
        v = c.value.value ? "true" : "false";
        break;
      case "textValue":
        v = c.value.value;
        break;
      case "enumOrdinal":
        v = `#${c.value.value}`;
        break;
    }
    bits.push(`${c.channelPath}=${v}`);
  }
  return bits.join(" · ") || (e.notes ?? "(no detail)");
}
