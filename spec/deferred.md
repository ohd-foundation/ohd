# OHD — deferred work / backlog

Consolidated punch-list of known-needed work explicitly deferred during the
beta cycle. Each entry carries enough context to pick it up cold. Supersedes
the old `connect/android/missing_features.md`.

Grouped by area. No priority ordering implied within a group.

---

## Storage core & schema

### ECG parent event — `measurement.ecg` session
Samsung ECG imports today as N `measurement.ecg_second` events (one per
second of waveform, 500 samples each) joined by a `correlation_id`, but with
no parent. Add a `measurement.ecg` parent (`top_level = true`) carrying
strip-level metadata (`correlation_id`, `classification`, `avg_heart_rate`,
`sampling_rate_hz`, `lead`, `seconds_count`, `device`, `software_version`,
`symptoms`). Children become `top_level = false`. The importer emits one
parent + N children atomically. History then shows one row per strip;
drill-down (EditEventScreen, already wired for correlation_id children)
surfaces the seconds. Mirrors the `food.eaten` / `intake.*` pattern.

### `display_name` column on `event_types`
Add a nullable `display_name` to the `event_types` table, populate via
migration 018 for every canonical type (`measurement.ecg_second` →
"ECG (1-second sample)", `activity.steps` → "Steps", …). Expose through
uniffi `EventTypeDto` + MCP `describe_data`. UI reads `display_name`
instead of humanizing the snake_case suffix.

### Channel unit refactor + marker semantics
Drop the `_g` / `_mg` / `_mcg` suffixes from channel paths — `carbs_g` →
`carbs` with the unit carried in channel metadata (the `channels.unit`
column already exists, currently unused as authoritative). Migration 018
renames the channel rows and adds `channel_aliases` rows so old payloads
still resolve.

Marker semantics fold in here: a writer may omit the `value` channel
entirely. `intake.caffeine` with `value=12, unit=mg` = known quantity;
`intake.caffeine` with no value (just `correlation_id`) = marker ("present,
amount unknown"). Readers sum only valued events and report a separate
marker count. `composition.*` already follows the marker pattern. Use case:
E330 / additives / "trace" nutrients with no gramage.

Touches every food writer/reader: `FoodDetailScreen`, `OpenFoodFacts`,
`HealthConnectSync`, MCP `log_food`, `aggregateIntakeChildren`,
`RecentEventsScreen`.

### Event-type registry — private/publish model
`custom.*` auto-registration already lands unknown types under
`custom.<original>` with transparent read-side fallback (done). Still
deferred: a per-user registry UI to view/rename custom types and a
publish/share path so a custom type can be promoted to a shared canonical
definition. Promotion of an existing `custom.X` row to canonical is a
one-line `UPDATE` (or a `type_aliases` insert) in a future migration.

---

## Health Connect

### Changes API for incremental sync
Current per-type sync watermark is `latest_stored.timestamp + 1ms`. If
Samsung Health drip-feeds older-timestamped samples into HC *after* a sync
runs, they land below the watermark and are silently skipped — the likely
cause of "new heart-rate samples never arrive". Switch to HC's
`getChangesToken()` / `getChanges(token)` API, which returns insertion-order
change events (inserts + deletes) regardless of record timestamp. Canonical
incremental-sync pattern for HC.

### Cycle tracking
Six record types skipped for v1 (Menstruation*, OvulationTest,
CervicalMucus, IntermenstrualBleeding, SexualActivity). Gate behind an
opt-in "cycle tracking" toggle so users who don't want it never see the
permission prompt.

### rc03 record types
`SkinTemperatureRecord`, `PlannedExerciseSessionRecord`,
`MindfulnessSessionRecord` — require `compileSdk 36` + AGP 8.9.1. Unblock
with the SDK platform bump.

---

## Wear OS companion app

New `connect/wear/` module — bypasses the Samsung Health → Health Connect
propagation lag (often hours) for live vitals. Reads HR / SpO2 / HRV / skin
temperature on the watch via the **Health Services API**
(`androidx.health.services:health-services-client`) — passive background
monitoring + foreground live mode. Pushes samples to the phone over the
**Wearable Data Layer** (`DataClient` / `MessageClient`), latency in
seconds. Phone-side `WearableListenerService` writes them to OHD storage as
`measurement.heart_rate` etc. with `source = watch_direct`; dedupe against
HC-sourced rows by preferring `watch_direct` on timestamp overlap.

Cost: new Wear module (build files, manifest, `BODY_SENSORS` permission,
capability declarations), ~200 LoC for the subscriber + publisher, phone-side
listener service, signing pipeline change (`<phone-id>.wear`). Standalone
watch app is simpler than a watch face. HC stays as the backstop for when
the watch is off-wrist / flat.

---

## Food module

### OFF search quality + caching
Flavoured / Zero product variants exist on OpenFoodFacts but search ranking
surfaces them poorly. Wanted: (a) cache OFF responses by `code` and by a
`<brand,name>` index so a prior scan short-circuits later searches;
(b) layered queries — OFF v2 product search first, then a looser-token
fallback. Note `cgi/search.pl` is flaky/rate-limited; a server-side cache
on our infra would also stabilise it.

### Add-missing-product flow
Scanning an unknown barcode is a dead end today. New screen: 404 → prompt
for name / brand / per-100g macros / package size / allergens. Save as a
local product (own-storage template) so future scans of the same code hit
our cache; optionally contribute back to OFF via their write API.

### Detail screen — product image
`FoodItem` needs an `imageUrl` field, `mapOffProduct` populates it from
OFF's `image_front_url`, `FoodDetailScreen` renders it at the top. Needs an
image loader (Coil dependency).

### Explicit favourites strip
The list under the scanner currently shows whatever was eaten — it should
be an *explicit*, curated favourites strip (add/remove, persisted like
`home_favourites_v1`). Tapping a favourite → detail; a clear "Add" button
quick-logs (prompts for grams if the amount is ambiguous). The today's log
moved to the History screen (done — see below).

### Full-nutrient detail + "other ingredients" inventory
`FoodDetailScreen` already renders the full macro/micro breakdown +
composition panel. Still wanted: an aggregate "everything you've taken in"
inventory view (all nutrients + all ingredients consumed over a period),
and verification that `composition.ingredient.*` markers populate for real
products after the migration-018 + custom.* changes.

---

## Medication

### Medication library + barcode lookup
`MedicationLibraryScreen` is a stub. Need an OFF-equivalent for pharma —
candidates: openFDA, DailyMed (US NDC), RxNorm (US NLM). Europe has no
single comparable open API. Barcode scan should read the GS1 DataMatrix on
pharma boxes.

---

## Profile

### Allergies & chronic conditions registry
No place to declare "allergic to X" / "have condition Y". Belongs in
Settings → Profile & Access. Two new event types `profile.allergy` /
`profile.condition` (`top_level = true`, state not time-series). Allergy:
name, severity (mild/moderate/anaphylactic), optional ICD-10/SNOMED,
last-reaction timestamp, notes. Condition: name, ICD-10, diagnosed-at,
active flag. Surface both on the Emergency screen and make them grantable
at break-glass time.

---

## MCP / agents

### Agent skills — MCP-native
Skills = natural-language "how to do X" procedures the model loads on
demand. Must work for every MCP-aware agent, not Claude-specific bundles.
Mount on the MCP `prompts/list` + `prompts/get` surface. `ohd-mcp-core`
grows a `skills/` module parallel to `tools/`, exposed through MCP
`prompts/*` server-side and a uniffi method on Android. Candidate skills:
lab-report summarisation, medication-interaction check, food/gastro
correlation, ECG-strip classification.

### CORD chat polish
SSE streaming (synchronous JSON ships today), retries on 5xx, OpenAI /
Gemini providers (stubbed), chat-history persistence (in-memory only).
Tool catalog is now full via `ohd-mcp-core` — no longer the 3-tool subset.

---

## Infrastructure

### api.ohd.dev OFF proxy is broken
The Caddy route for `/v1/openfoodfacts/*` serves OFF's HTML error page for
every path (barcode + search alike). The Android app currently routes
around it — calls OpenFoodFacts directly. Either fix the Caddy route (and
add the response cache the proxy was meant to provide) or remove the route
+ the dead `searchProxy`/`fetchProxy` code paths in `OpenFoodFacts.kt`.

### SaaS-side wiring
Real OIDC verification on `/v1/account/oidc/link` (today trusts
`provider + sub` from the client). Stripe wiring on
`/v1/account/plan/checkout` (placeholder URL today). Server-side
rate-limiting on `/v1/account` registration.

### connect/mcp/ Python — delete
The Python MCP server is superseded by `connect/mcp-rs/` (Rust, deployed at
mcp.ohd.dev). Delete `connect/mcp/` once external Claude Desktop configs
pointing at the Python stdio have been migrated.

---

## Code hygiene

### Named types over bare strings
`String` / `Map<String,String>` / `Vec<(String,String)>` lose intent at
call sites. Add named aliases (`type Ulid26 = String`,
`type ChannelPath = String`, `typealias ToolName = String`, …). Zero
runtime cost. Apply where the string is "load-bearing"; skip the obvious
(`Int.toString()`).

### Custom-form runtime polish
`CustomFormFillScreen`: scroll the first missing field into view on a
failed save (`LazyListState` + `bringIntoView`); drag-to-reorder fields on
the builder (currently up/down buttons).

---

## Auth & onboarding

- "Forgot recovery code?" affordance once an OIDC identity is linked.
- Account deletion flow (server-side delete + local wipe).

---

## Misc UX

- Better empty-state copy across History / Food when there are zero events.
- Edit affordance for past `food.eaten` events that re-derives the
  `intake.*` children.
- Pre-beta hardening pass: comprehensive event-type DB review, cleanup of
  legacy flat-macro channels on `food.eaten`, schema audit.

---

## Done — moved off the backlog

For reference, completed during the beta cycle and removed from this list:

- Tool-catalog consolidation — `ohd-mcp-core` shared Rust crate + dispatch,
  consumed by the Android uniffi shim and the `ohd-mcp-rs` server.
- `ohd-mcp-rs` MCP server built + deployed at mcp.ohd.dev.
- `custom.*` auto-registration with transparent read-side fallback.
- Migration 018 — registered all Health Connect + `intake.*` event types
  (food macros and ~22 HC types were silently rejected before).
- `count_events` honours the `visibility` filter (home count was including
  detail children).
- Filterable + charted History screen — range selector, event-type chips,
  per-metric day/week/month charts; absorbs the food daily-log, glucose
  chart, and HR-over-day into one screen.
