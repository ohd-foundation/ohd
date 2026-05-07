# NutriGauge — Implementation Notes

Component ID in Pencil: `xEama`

## Visual structure

Thin donut ring (~4 px) on a 76×76 canvas. Two concentric ellipses (background + arc) plus two text nodes stacked in the center.

| Layer | Purpose |
|---|---|
| bg ellipse (`J50lrU`) | Full-circle track, always visible. Color changes by state (see below). |
| arc ellipse (`sBGiB`) | Filled portion. `startAngle: 90` (12 o'clock), `sweepAngle` negative = clockwise. `innerRadius: 0.89`. |
| value text (`n0c5V5`) | Percentage, e.g. `"66%"`. JetBrains Mono 11 px. Color by state. |
| gramRow frame (`zn1fY`) | Horizontal layout, center-justified, no gap. |
| ↳ gramVal (`c4amEU`) | Gram value, e.g. `"73g"`. Color by state. |
| ↳ gramTgt (`N0yg6f`) | Daily target, e.g. `"/110g"`. Always `#9B9B9B`. |

## State rules

### Normal (30 % – 100 %)
- bg: `#EBEBEB`
- arc fill: `#ABABAB`
- text/gramVal fill: `#ABABAB`
- sweepAngle: `-(pct / 100) * 360`

### Low (< 30 %)
- bg: `#EBEBEB`
- arc fill: `#0A0A0A`
- text/gramVal fill: `#0A0A0A`
- sweepAngle: `-(pct / 100) * 360`

### Over limit (> 100 %)
- bg: `#FADCDD` (muted red — do NOT use a fully-saturated red; `#EBEBEB` mixed ~15 % with `#E11D2A`)
- arc fill: `#E11D2A`
- text/gramVal fill: `#E11D2A`
- sweepAngle: `-((pct % 100) / 100) * 360`  ← **modulo 100**, so the ring stays readable at any multiple
- The displayed percentage is the **real** value (e.g. `"140%"`), not the modulo value. Only the arc sweep is modulo.
- At exactly 100 % the ring is full (`-360`). Above 100 % the ring resets and sweeps again from the top.

## Default nutrients

Carbs · Protein · Fat · Sugar. Daily targets configurable in Settings per user.

## Rich text note

`gramVal` and `gramTgt` are intentionally separate nodes so each can carry its own color. In a native implementation use a single `AttributedString` / `SpannableString` / `NSAttributedString` span rather than two sibling views if layout constraints allow.
