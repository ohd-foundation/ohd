# Future implementation: Gemini App Functions

> Expose OHDC operations (log food, query events, log symptom, etc.) to
> on-device Gemini through Android's **App Functions** API, so the user can
> ask Gemini "log a banana" or "what's my pulse trend" and Gemini calls
> directly into the app — no extra round trip, no copying data into another
> assistant context.

## Status — deferred, blocked on Android SDK bump

App Functions is in **experimental preview** on Android 16. To adopt it the
Connect app needs:

- `compileSdk = 36` (current branch is `34`).
- AGP 8.9.x (current branch is 8.6.1).
- `androidx.appfunctions:appfunctions` dependency + KSP annotation
  processor.
- `<uses-permission android:name="android.permission.EXECUTE_APP_FUNCTIONS" />`.
- Runtime target Android 16 — older devices keep using
  [App Actions / Shortcuts](../../../connect/android/app/src/main/res/xml/shortcuts.xml)
  as the Gemini-callable surface.

Until that ships, the App Actions capability in `shortcuts.xml` is the
primary Gemini integration. The marker action proves the round trip; once
we bump the SDK the per-domain functions described below replace it.

## What this doc is for

Reserving the design space so v1's `StorageRepository` surface and the
Compose call sites don't have to be re-architected when App Functions
lands. The key claims to validate:

- The dispatch layer (`StorageRepository`) is already
  `Result`-returning, suspend-friendly, and side-effect-isolated, so
  wrapping each public method in `@AppFunction` is mechanical.
- The OHDC primitives (`put_events`, `query_events`, `audit_query`, etc.)
  are the right granularity for Gemini to call — one function per OHDC
  RPC, not finer. Gemini already handles parameter extraction.
- The local-first contract holds: on-device storage stays the path App
  Functions writes through; OHD Cloud users get the same operations
  shipped over the remote backend (the backend selector already abstracts
  this).

## Design-space sketch

### Surface — one App Function per OHDC operation

A single Kotlin file `OhdAppFunctions.kt` in the app, each method
annotated `@AppFunction(isDescribedByKDoc = true)` so Gemini learns the
contract from KDoc:

```kotlin
class OhdAppFunctions(private val repo: StorageRepository) {

    /**
     * Log a food eaten by the user. The standard nutrition channels
     * (kcal, carbs_g, protein_g, fat_g, sugar_g) are filled by the
     * resolver when not supplied.
     *
     * @param name human-readable name of the food, e.g. "banana".
     * @param grams how many grams were eaten.
     * @param atMs unix-millis the user actually ate it; defaults to now.
     */
    @AppFunction(isDescribedByKDoc = true)
    suspend fun logFood(
        appFunctionContext: AppFunctionContext,
        name: String,
        grams: Double,
        atMs: Long = System.currentTimeMillis(),
    ): LogFoodResult { /* … */ }

    /** Query recent events under a self-session scope. */
    @AppFunction(isDescribedByKDoc = true)
    suspend fun queryEvents(
        appFunctionContext: AppFunctionContext,
        eventType: String,
        fromMs: Long? = null,
        toMs: Long? = null,
        limit: Int = 50,
    ): List<EventSummary>

    // log_symptom, log_measurement, log_medication, log_mood, log_sleep,
    // log_exercise, log_free_event — one per quick-log domain.
    // grant_summary, create_grant, list_pending, approve_pending — one per
    // operator-side OHDC RPC. Catalog mirrors `connect/mcp/`.
}
```

Returns + parameter types are `@AppFunctionSerializable` data classes so
Gemini can render them.

### Source of truth — `ohd-mcp-core`, eventually

`storage/crates/ohd-mcp-core/` already owns the canonical tool catalog
(see `connect/android/missing_features.md` and the MCP-placement plan).
When that crate exposes a stable Kotlin-callable façade via uniffi,
`OhdAppFunctions` becomes a thin annotation layer over it — the same
catalog drives:

- the on-phone CORD chat (uniffi → `ohd_mcp_core.execute_tool`),
- the remote MCP server at `mcp.ohd.dev` (axum → `ohd-mcp-core`),
- App Functions for Gemini (annotated Kotlin → `ohd_mcp_core`).

Then there is one tool surface, three transports, and Gemini gets every
OHDC RPC for free.

### Privacy + scope

App Functions invocations are user-authorised through the system. They
run under the OHD Connect process so the existing self-session token
(local or OHD Cloud) is the auth context — no extra grant flow. The
audit log records each App Function call the same way it records any
write or query (`actor_type = "app_function"` ride-along TBD).

For OHD Cloud, network-bound calls Gemini makes through App Functions go
over the same remote backend as the UI; the storage server has no
special case for App Functions.

### Discovery + dev workflow

`adb shell cmd app_function list-app-functions` enumerates what an app
exposes. Connect should ship a settings card "Gemini capabilities" that
mirrors the registered functions for the user — full disclosure of what
Gemini can do on their behalf.

## Cross-references

- App Actions / Shortcuts (current Gemini surface) —
  [`../../../connect/android/app/src/main/res/xml/shortcuts.xml`](../../../connect/android/app/src/main/res/xml/shortcuts.xml)
- MCP placement plan (the `ohd-mcp-core` deliverable) —
  [`../../../connect/android/missing_features.md`](../../../connect/android/missing_features.md)
- CORD on-phone tool surface (Anthropic tool-use over the same dispatch) —
  [`../../../connect/android/app/src/main/java/com/ohd/connect/data/CordTools.kt`](../../../connect/android/app/src/main/java/com/ohd/connect/data/CordTools.kt)
- OHDC channels + event model — [`../design/storage-format.md`](../design/storage-format.md)
