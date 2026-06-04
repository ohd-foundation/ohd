# OHD backlog

> Lightweight tracker for items raised across recent sessions that aren't
> in flight. The full design-heavy entries live under
> `spec/docs/future-implementations/`; this file is the index + status so
> nothing gets lost between rounds.

Status:
- 🟢 in production
- 🟡 partial — some pieces shipped, more queued
- 🟥 not started
- 🛈 design captured; no code

Sorted within each area by rough impact.

---

## Connect app — UX

### History / log search

- 🟡 **Aggregate / chart surface (separate from History log).** Day / week
  / month / year + custom range, per-channel charts via the existing
  `Aggregate` RPC, Food-tab "today" panel lifted to a reusable component
  bucketed per day on week+ views. Chart helpers (`EventChart`,
  `numericSeries`, `bucketByDay`, `chartXLabels`) stay in
  `RecentEventsScreen.kt` ready to move; the `TimeRange` import is held
  for them.
  → `spec/docs/future-implementations/history-and-aggregates.md`
- 🟡 **Food tab — per-day aggregate rows in week+ views.** Today =
  per-meal list as today; week / month / year = one row per day carrying
  that day's kcal + macros, tappable to drill into per-meal rows.
- 🛈 **Visibility / event-type customization.** Per-family default
  visibility (`food.*`, `intake.*`, `medication.*`, …) with user
  overrides; canonical measurement-type registry as the source of truth
  for "what types exist". Replaces the current hard-coded
  `eventTypesNotIn(FOOD_EVENT_TYPES)` + `top_level` placeholder.
  → `spec/docs/future-implementations/history-and-aggregates.md`
- 🟥 **Future-date History rows.** Today returns "No entries on this
  day" cleanly. Open question whether logging *scheduled* events
  (future meds / appointments) should surface here.

### Food

- 🟥 **Edit / remove custom foods.** `CustomFoodStore` has `remove(id)`
  + persistence; the UI only adds. Need a long-press / detail-screen
  affordance to manage existing custom foods.
- 🟥 **Upstream submission of custom foods — OFF contribution path.**
  Legal note: OFF data is **ODbL** (Open Database License). It requires
  attribution + share-alike for *derived databases*; reading the OFF
  API for our own users' personal logs is **not** a derived database
  redistribution, so we're not legally obligated to contribute back.
  But it's good citizenship — a barcode the user scans + custom-creates
  is almost certainly missing from OFF too, and uploading it benefits
  everyone. Two-stage rollout:
   - **Phase 1 — opt-in upload button** on the saved custom food's
     detail screen ("Contribute to OpenFoodFacts"). Hits OFF's product
     contribution API (`/cgi/product_jqm2.pl` or the newer one) with
     the user's OFF account (or anonymous if they don't have one).
     Photos required for many fields per OFF's quality rules — gate
     the button on having `barcode + name + at least one photo`.
   - **Phase 2 — keep-local-until-accepted**: per the user's original
     flag, the local custom keeps being served until OFF acceptance
     comes back, at which point the local override is dropped on next
     search-refresh. Requires polling / a callback flow; deferable.
  Either way: add an "Upload to OpenFoodFacts" entry to the
  custom-food detail / edit screen as the visible surface.
- 🟥 **Photos on `FoodItem` + custom-food form.** OFF stores
  `image_url`, `image_ingredients_url`, `image_nutrition_url`,
  `image_packaging_url`. Mirror as `FoodItem.photos: List<FoodPhoto>`
  with `kind ∈ {product, ingredients, nutrition, packaging}` + a URI
  (file:// for local-only customs; https:// when populated from OFF).
  Form gets a "Photos ▾" expander with a Camera / Gallery action that
  drops a 1280-px-max compressed JPEG under `app/files/food-photos/`
  and stores the file:// uri on the row. Required for the OFF-upload
  button above. (Touches `FoodItem` + serializer + a `v3→v4` store
  migration; relates to the packaging-info item above which already
  needed `v2→v3`, so the two should land together if possible.)
- 🟢 **Packaging info on `FoodItem` + custom-food form.** Shipped
  beta76: `Packaging(material, format, recyclable, recycledContentPct,
  notes)` data class, `FoodItem.packaging: Packaging?`, "Packaging ▾"
  expander in `FoodCreateScreen` with material × format × recyclable
  pill rows + recycled-content % numeric input + notes text area,
  serializer extended, store bumped `custom_foods_v2 → v3` (v1 + v2
  rows migrate forward through the same parser since every added field
  is optional with a default).
- 🟥 **List-input UX upgrade — "Add an entry…" picker + freeform.**
  The current allergens / traces / ingredients-analysis / labels are
  flat multi-select chip rows over a fixed token set. That works for
  the common 14 EU allergens but breaks down everywhere else:
   - Ingredients + additives are *open* lists — the user paste-and-edits
     comma-separated strings today, which is clumsy.
   - Even allergens / traces want "other" cases (sulphites-not-listed,
     regional specifics).
   - Forced-pill UX hides what's selected once the row scrolls.
  Target shape, same for every list field:
   - A chip row showing the **currently-selected** entries (tap to
     remove). One row, wraps; the user always sees what's in.
   - A trailing **"+ Add"** chip that opens a small bottom sheet with
     (a) a select / search over the canonical token list for that
     field, and (b) a freeform text input at the bottom for
     "Something else: ___". Confirm adds the entry to the chip row.
  Applies to: allergens, traces, ingredientsAnalysis, labels,
  ingredients, additives. The canonical lists stay where they are
  (per-field constants in `FoodCreateScreen`); the sheet is a shared
  component.
- 🟥 **AI-assisted custom-food entry — text → structured form fill.**
  The user feeds the model whatever they have — back-of-pack text,
  product description from a website, recipe-blog paragraph, voice
  transcript, OFF-page paste — and the model parses it into every
  `FoodItem` field at once. Two shapes:
   - **In-form "Fill from description" affordance.** A multi-line
     text area at the top of `FoodCreateScreen` ("Paste a description,
     ingredients list, or back-of-pack text…") + a button that sends
     the blob to the active CORD agent with a prompt asking for a
     JSON `FoodItem`. Returned fields populate the form below; the
     user reviews + edits + Saves. Must **never overwrite** fields the
     user has already filled in by hand — the AI is additive.
   - **From CORD chat directly.** A `create_custom_food` tool added
     to `ohd-mcp-core`'s catalog so the user can say *"create:
     homemade granola, oats + honey + almonds, ~450 kcal/100 g,
     glass jar"* in chat and the agent assembles a `FoodItem` and
     writes through the same `CustomFoodStore.add` the form uses.
   Both paths share the same JSON shape the LLM emits — keep the
   parser/validator in one place (likely a small module sitting
   alongside `CustomFoodStore`).
- 🟢 **Nutrition targets — personalized, overridable, meaningful defaults.**
  Shipped: `NutritionGoalsStore` (Mifflin–St Jeor BMR × PAL × goal scale,
  with per-macro overrides and WHO fallback), `Settings → Nutrition goals`
  editor screen, and `FoodScreen` now reads `effectiveTargets(ctx)` instead
  of the hard-coded 2000 / 110 / 80 / 70 / 20.
- 🟥 **Search-by-name "leaves the activity".** Flagged a few rounds
  ago; suspected the inline CameraX preview reopening after popBackStack.
  Worth re-testing on beta71+; if it still repros, dig in.

### Settings / setup

- 🟥 **Third "OHD Cloud" share card duplicated for the *picker*.**
  Storage picker already has the cloud option; the `share` flow now has
  cloud-direct (beta70). The original "third card" idea was a unified
  treatment across both surfaces — partly satisfied, design-level review
  could fold them into one component family.

## Server / protocol

- 🟥 **ListSources RPC** (`SELECT source, COUNT(*) FROM events GROUP BY
  source`). Counterpart to `CountSources`; lets the Sources screen show
  per-source breakdowns (e.g. `health_connect : 17 824 · manual : 42`).
- 🟥 **Aggregate RPC plumbed to Kotlin.** The RPC exists server-side
  but isn't exposed through uniffi yet — needed by the future aggregate
  surface and the per-day food aggregates.
- 🟥 **Grant-scoped variants of `ListEventTypes` / `CountEvents` /
  `CountSources`.** Self-session-only today; the spec already calls out
  intersecting with `grants.aggregation_only` / per-type rules.
- 🟥 **Delete-events forward migration mechanism.** The `suspended_at_ms`
  fix had to be a one-off ALTER on the live DB because the migration
  runner rejects duplicate-column errors. Teach the runner to soft-skip
  `duplicate column name` so future schema edits can ship a migration
  file safely.
  → `feedback_server_migration_in_place.md` memory.

## Storage / portability

- 🟢 **Portable JSONL export** (beta64) — Settings → Export → Download.
- 🟢 **Portable JSONL import** (beta65) — Settings → Export → Import.
  Idempotent via `(source, source_id)` dedup.
- 🟥 **Cross-implementation DB transfer.** Formalised pipeline +
  signed-archive variant of the JSONL flow; today the JSONL bridge is
  enough for a single user, but a clinic moving between operators wants
  something stronger.

## SaaS plans

- 🟡 **Plan card** in Storage Settings (beta62) shows tier + limits +
  Upgrade stub. Reads from local `OhdAccountStore`; the
  `/v1/account/plan` server endpoint is wired but the app doesn't hit
  it yet.
- 🟥 **Server-side retention enforcement.** `PlanInfo` declares limits
  (Free: 7 days / 25 MB; Paid: unlimited / 5 GB). A sweeper or
  write-side check needs to actually apply them on the storage side.
- 🟥 **Stripe checkout.** `POST /v1/account/plan/checkout` returns a
  stub URL pointing at the roadmap; the real Stripe flow + plan
  upgrade lands later.

## Gemini / AI surface

- 🟢 **App Action: real `LOG_FOOD`** (beta60). Long-press / adb /
  Google Assistant via `actions.intent.CREATE_THING` capability.
  Routing through Gemini in production needs Play Store + Actions
  Console linkage.
- 🟡 **Full App Functions (Android 16+).** Spec captured; blocked on
  compileSdk 36 + AGP 8.9.x + Android 16 runtime + the experimental
  `androidx.appfunctions` library. SDK install was attempted and
  reverted.
  → `spec/docs/future-implementations/gemini-app-functions.md`
- 🟥 **Per-domain App Actions / Functions** beyond `LOG_FOOD` —
  `log_symptom`, `log_measurement`, `query_events`, etc. Each gets a
  parameter-aware shortcut once the catalog is unified through
  `ohd-mcp-core`.

## CORD / data link

- 🟢 **Connection rename** (cord redeploy). Inline edit on the
  Connection page + `PATCH /v1/sources/:id` + DB column.
- 🟢 **Cloud-direct share** (beta70 + cord redeploy). `ohd://share/cloud`
  parsed cleanly; route forwards to the existing `kind=direct` path.
- 🟥 **MCP placement plan — `ohd-mcp-core` driving everything.** The
  Rust crate exists; the standalone `mcp.ohd.dev` axum binary is the
  remaining piece per the original plan. Today both surfaces (local
  uniffi + remote OHDC RPCs `ListTools`/`ExecuteTool`) call the same
  catalog, so the work left is the third transport.
  → `connect/android/missing_features.md` (task #7).

## Relay / live channels

- 🛈 **Live channel subscriptions** (push-to-subscriber pattern). Spec
  captures the single-event vs bulk fan-out semantics the user added.
  Not on a roadmap yet; ride the relay's existing tunnel + push-wake
  primitives when adopted.
  → `spec/docs/future-implementations/live-channel-subscriptions.md`

---

## Recent shipped (sanity-check during your review)

| beta | What |
|---|---|
| 56 | App-wide off-main sweep |
| 57 | Bulk-log + HC perm diagnostic |
| 58 | Delete-remote-data |
| 60 | Gemini LOG_FOOD action |
| 61 | CORD-on-OHD-Cloud (`ListTools` / `ExecuteTool`) + `CountEvents` |
| 62 | SaaS Plan card |
| 63 | History stopgap (Year actually 365d; 10k chip scan) |
| 64 | JSONL export + custom foods |
| 65 | `ListEventTypes` RPC + JSONL import |
| 66 | History chips via `ListEventTypes` + count badges |
| 67 | History single-day picker |
| 68 | Audit-log fallback removed |
| 69 | `list_event_types` honours visibility + food excluded from History |
| 70 | **Cloud-direct share** (`ohd://share/cloud` end-to-end) |
| 71 | `CountSources` RPC + Home source-count tile |

Touch each surface you care about; jot whatever's broken or off back to me
and I'll either fix or add to the right section above.
