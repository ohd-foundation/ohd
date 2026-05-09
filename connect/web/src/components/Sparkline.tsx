/**
 * Inline SVG sparkline. No chart library — keeps the bundle slim and
 * matches `care/web`'s no-dependency aesthetic. When ReadSamples wires we'll
 * graduate to per-channel mini-charts.
 */
export function Sparkline({ points }: { points: number[] }) {
  if (points.length === 0) return null;
  const w = 600;
  const h = 60;
  const min = Math.min(...points);
  const max = Math.max(...points);
  const span = max - min || 1;
  const dx = w / Math.max(points.length - 1, 1);
  const path = points
    .map(
      (v, i) =>
        `${i === 0 ? "M" : "L"} ${(i * dx).toFixed(1)} ${(h - ((v - min) / span) * (h - 8) - 4).toFixed(1)}`,
    )
    .join(" ");
  return (
    <svg viewBox={`0 0 ${w} ${h}`} className="sparkline" preserveAspectRatio="none" aria-hidden="true">
      <path d={path} fill="none" stroke="var(--color-accent)" strokeWidth="1.5" />
    </svg>
  );
}
