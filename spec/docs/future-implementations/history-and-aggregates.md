# Future implementation: History UX + aggregates

> Replace the current "Today/Week/Month/Year" segmented list with a
> filter-first History and a stat-first Home, and add per-period food
> aggregates.

## Status — deferred, queued

The current History screen pages a single fixed time range and renders
every event as a row. That model collapses in the presence of any heavy
data source (Health Connect's step / heart-rate flood pushes low-frequency
types — glucose, blood pressure — out of the visible window). The
temporary `limit = 10 000` stopgap in `RecentEventsScreen.kt` is a band-aid
until this redesign lands.

## Home tab — stats, not rows

The day/week/month/year selector stays. What changes is **what gets
displayed under it**:

- **Event count** for the range (already shipped via the `CountEvents` RPC).
- **Source count** — distinct producers that contributed events in the range
  (the user's phone, Health Connect, paired wearables, …). Needs a small
  helper RPC (`CountSources(filter)`) or a `SELECT COUNT(DISTINCT source)`.
- **Aggregates** computed on the storage side via the existing `Aggregate`
  RPC and a curated set of OHDC-defined aggregates per common channel:
  - `sum(activity.cycling.distance_m)` → "12.4 km cycled this week".
  - `min/max/mode(measurement.heart_rate.value)` → "HR 48–148, mode 64
    this week".
  - `sum(intake.kcal.value)` → "16 200 kcal this week".
  Each aggregate is one tile; the active range drives the time bounds.
  The selection of which aggregates to display is user-customisable via a
  Settings "Home dashboard" pane (later).

Year keeps existing as a range — useful for the macro stats — even though
nobody scrolls a list of a year's events.

## History split — log search vs aggregates

The day / week / month / year selector was carried over from the Home
tab by inertia — it makes sense for stat tiles but **not for a flat
event list**. "Show me a year of events" isn't a useful surface; "show
me a year of glucose values as a chart" is. So History splits into two
distinct surfaces:

### 1. Event log (search the log by date / range)

A flat list of individual events. The unit is one event row.

- Default to **a single day**, today.
- Future: extend to an arbitrary `from–to` range so the user can search
  the log over any span ("show me everything between Mar 1 and Mar 14
  last year"). The list always paginates; the goal is to *find* a
  specific event, not to see trends.
- The `ListEventTypes(filter)` chip set still applies — pick a type to
  scope the list further.
- This is the entry point for an existing event the user wants to
  inspect or edit (the `findEventByUlid` lookup is what drives that
  flow today).

### 2. Aggregates / visualizations (the chart view)

A separate screen, **not** an event list — the unit here is one chart /
one aggregate tile.

- Keeps the day / week / month / year / custom selector (it's what
  aggregates need).
- Per-channel charts (heart rate over time, glucose trend, cycling
  distance) computed via the existing `Aggregate` RPC with a bucket
  appropriate to the range.
- Lifts the existing food panel logic (FoodScreen's "Today" macros
  panel) into a reusable component so the same kcal/macros aggregation
  shows on this surface bucketed per day in week+ views.

These two surfaces eventually split into separate routes / tabs. For now
the simpler-of-the-two — the log search — replaces today's History;
the aggregate view is its own follow-up pass.

## Food tab — per-day aggregate above the row list

The Food tab's "TODAY" section already shows a kcal-and-macros panel.
Generalise it:

- **Sub-day views** (Today): keep the per-meal row list as today, with
  the running daily totals at the top.
- **Week / month / year views**: replace the per-meal rows with **one
  row per day** carrying that day's aggregate (kcal + macros). Tapping
  a day-row drills into the per-meal list for that day.

This makes the Food tab a real food diary at any timescale, mirroring
the same `Aggregate` RPC the Home tab uses but bucketed by `DATE_TRUNC
('day', timestamp_ms)`.

## Required server-side pieces

- `ListEventTypes(filter) → repeated {name, count}` — distinct event
  types within a filter, with counts. Cheap GROUP BY on the same
  predicates `QueryEvents` / `CountEvents` honour.
- `CountSources(filter) → int` — distinct source count in range.
  (Cheaper than a per-source breakdown; the breakdown can come later.)
- The existing `Aggregate(channel, op, bucket?)` RPC covers the Home /
  Food aggregates; the client just calls it per-tile with the active
  range.

All three are self-session-only for v0; grant-scoped aggregation rides
the existing `aggregation_only` flag without changes.

## Cross-references

- Today's stopgap (10 000-row scan) —
  [`../../../connect/android/app/src/main/java/com/ohd/connect/ui/screens/RecentEventsScreen.kt`](../../../connect/android/app/src/main/java/com/ohd/connect/ui/screens/RecentEventsScreen.kt)
- Existing aggregate RPC — `storage/proto/ohdc/v0/ohdc.proto::Aggregate`
- Home count tile — `connect/android/app/src/main/java/com/ohd/connect/ui/screens/HomeScreen.kt`
