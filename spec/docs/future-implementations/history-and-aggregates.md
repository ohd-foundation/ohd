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

## History tab — filter-first

Replace the segmented Today / Week / Month / Year strip with:

- A **date range picker** at the top. Default = today; the user can drag
  the lower / upper bound to any custom window (week, month, year,
  arbitrary).
- An **event-type filter** below — the chip set is driven by the
  `ListEventTypes(filter)` RPC (returns `{name, count}` pairs scoped to
  the chosen date range). Cheap on the server (`SELECT event_type,
  COUNT(*) FROM events WHERE … GROUP BY event_type`). Replaces the
  10 000-row client-side scan in `RecentEventsScreen` today.
- A **paged event list** for the active filter. Small page (~100 rows)
  with a "Load more" trigger at the bottom — no more pre-fetching 10 000
  rows just to render the first screen.

When **exactly one event type** is selected and the channel structure is
chartable (single real value over time, e.g. `measurement.heart_rate`,
`measurement.blood_glucose`, `intake.kcal`), the screen renders a chart of
that channel over the chosen range *above* the list. The list stays
below so the user can drill into individual rows. This is where the
"visualization" the user mentioned lives.

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
