interface MetricTileProps {
  label: string;
  value: number | string;
  /** Optional secondary text under the value (units, delta, etc.). */
  hint?: string;
  /** Severity tint for the tile border. */
  tone?: "neutral" | "alert" | "warn" | "success";
}

export function MetricTile({ label, value, hint, tone = "neutral" }: MetricTileProps) {
  return (
    <div className={`metric metric-${tone}`}>
      <div className="metric-label">{label}</div>
      <div className="metric-value">{value}</div>
      {hint && <div className="metric-hint">{hint}</div>}
    </div>
  );
}
