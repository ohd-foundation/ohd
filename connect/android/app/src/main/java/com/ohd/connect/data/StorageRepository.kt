package com.ohd.connect.data

import android.content.Context
import java.io.File
import uniffi.ohd_storage.AuditEntryDto
import uniffi.ohd_storage.AuditFilterDto
import uniffi.ohd_storage.CaseDetailDto
import uniffi.ohd_storage.CaseDto
import uniffi.ohd_storage.CaseStateDto
import uniffi.ohd_storage.ChannelValueDto
import uniffi.ohd_storage.CreateGrantInputDto
import uniffi.ohd_storage.EmergencyConfigDto
import uniffi.ohd_storage.EventDto
import uniffi.ohd_storage.EventFilterDto
import uniffi.ohd_storage.EventInputDto
import uniffi.ohd_storage.GrantDto
import uniffi.ohd_storage.GrantEventTypeRuleDto
import uniffi.ohd_storage.GrantSensitivityRuleDto
import uniffi.ohd_storage.GrantTokenDto
import uniffi.ohd_storage.GrantUpdateDto
import uniffi.ohd_storage.ListGrantsFilterDto
import uniffi.ohd_storage.OhdStorage
import uniffi.ohd_storage.PendingEventDto
import uniffi.ohd_storage.PutEventOutcomeDto
import uniffi.ohd_storage.RetroGrantInputDto
import uniffi.ohd_storage.TrustedAuthorityDto
import uniffi.ohd_storage.ValueKind

/*
 * NOTE — uniffi prerequisites.
 *
 * This file calls into `package uniffi.ohd_storage`, which is regenerated
 * from `storage/crates/ohd-storage-bindings/src/lib.rs`. Until BUILD.md's
 * Stage 1 + Stage 2 have been run, the imports above won't resolve and
 * Gradle will refuse to build. The recipe in short:
 *
 *   # Stage 1: cross-compile the Rust core to per-ABI .so files
 *   cd storage/crates/ohd-storage-bindings
 *   cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
 *       -o ../../../connect/android/app/src/main/jniLibs build --release
 *
 *   # Stage 2: emit the Kotlin façade
 *   cd ../..   # storage/
 *   cargo run --features cli --bin uniffi-bindgen -- \
 *       generate \
 *       --library target/release/libohd_storage_bindings.so \
 *       --language kotlin \
 *       --out-dir ../connect/android/app/src/main/java/uniffi
 *
 * After Stage 2 the `uniffi.ohd_storage.*` imports above resolve and this
 * file links cleanly. Both `app/src/main/jniLibs/` and
 * `app/src/main/java/uniffi/` are gitignored — every contributor regens
 * locally. See `connect/android/BUILD.md` for the full recipe.
 *
 * Method-name mapping — Rust snake_case → Kotlin camelCase (uniffi 0.28
 * convention):
 *
 *   open(path, key_hex)               → OhdStorage.open(path, keyHex)
 *   create(path, key_hex)             → OhdStorage.create(path, keyHex)
 *   put_event(input)                  → handle.putEvent(input)
 *   query_events(filter)              → handle.queryEvents(filter)
 *   issue_self_session_token()        → handle.issueSelfSessionToken()
 *   list_grants(filter)               → handle.listGrants(filter)
 *   create_grant(req)                 → handle.createGrant(req)
 *   revoke_grant(ulid, reason)        → handle.revokeGrant(ulid, reason)
 *   update_grant(ulid, update)        → handle.updateGrant(ulid, update)
 *   list_pending()                    → handle.listPending()
 *   approve_pending(ulid, alsoTrust)  → handle.approvePending(ulid, alsoTrust)
 *   reject_pending(ulid, reason)      → handle.rejectPending(ulid, reason)
 *   list_cases(state_filter)          → handle.listCases(stateFilter)
 *   get_case(ulid)                    → handle.getCase(ulid)
 *   force_close_case(ulid)            → handle.forceCloseCase(ulid)
 *   issue_retrospective_grant(...)    → handle.issueRetrospectiveGrant(...)
 *   audit_query(filter)               → handle.auditQuery(filter)
 *   get_emergency_config()            → handle.getEmergencyConfig()
 *   set_emergency_config(cfg)         → handle.setEmergencyConfig(cfg)
 *   register_signer(...)              → handle.registerSigner(...)
 *   list_signers()                    → handle.listSigners()
 *   revoke_signer(kid)                → handle.revokeSigner(kid)
 *   export_all()                      → handle.exportAll()
 *   format_version()                  → handle.formatVersion()
 *   protocol_version()                → handle.protocolVersion()
 *   user_ulid()                       → handle.userUlid()
 */

/**
 * In-app domain model for one event row.
 *
 * Mirrors `EventDto` from the uniffi surface but omits the FFI-only kind
 * discriminator (`ValueKind`) — channel scalars are flattened to a
 * `String` for display. The full structured form is reconstituted on the
 * way out via `StorageRepository.putEvent`.
 */
data class OhdEvent(
    val ulid: String,
    val timestampMs: Long,
    val durationMs: Long?,
    val eventType: String,
    val channels: List<OhdChannel>,
    val notes: String?,
    val source: String?,
    /**
     * `true` for entry-level events (what Recent / History / home count
     * surface by default). `false` for detail rows like `intake.*` under a
     * food.eaten or `measurement.ecg_second` under an ECG session.
     */
    val topLevel: Boolean = true,
)

data class OhdChannel(
    val path: String,
    /** Pre-rendered string form. The structured value is in [scalar]. */
    val display: String,
    val scalar: OhdScalar,
)

sealed interface OhdScalar {
    data class Real(val v: Double) : OhdScalar
    data class Int(val v: Long) : OhdScalar
    data class Bool(val v: Boolean) : OhdScalar
    data class Text(val v: String) : OhdScalar
    data class EnumOrdinal(val ordinal: kotlin.Int) : OhdScalar
}

/** Outcome of [StorageRepository.putEvent]. */
sealed interface PutEventOutcome {
    data class Committed(val ulid: String, val timestampMs: Long) : PutEventOutcome
    data class Pending(val ulid: String, val expiresAtMs: Long) : PutEventOutcome
    data class Error(val code: String, val message: String) : PutEventOutcome
}

/**
 * Repository facade over `OhdStorage`. Singleton — the underlying
 * `OhdStorage` Arc is thread-safe (every uniffi method serialises through
 * the storage core's mutex), so a single global instance is fine.
 *
 * Initialisation is two-step:
 *   1. [init] is called from `MainActivity.onCreate` with the app context.
 *   2. [openOrCreate] (or [open]) is called once the user has finished the
 *      setup screen.
 *
 * Repository methods that need the handle call [requireHandle], which
 * throws `IllegalStateException("Storage not opened…")` if the user
 * hasn't completed setup. Compose call sites all wrap in `runCatching`
 * so an error surfaces as `Result.failure` rather than a crash.
 */
object StorageRepository {

    private var appContext: Context? = null

    /**
     * The live uniffi handle. Populated by [openOrCreate] / [open].
     * `null` until the Setup screen finishes — every method that needs
     * it calls [requireHandle].
     */
    private var handle: OhdStorage? = null

    fun init(context: Context) {
        appContext = context.applicationContext
    }

    private fun requireHandle(): OhdStorage =
        handle ?: error("Storage not opened — finish the Setup screen first.")

    /**
     * Run `block` against the open uniffi handle, wrapping the result in a
     * [Result] so call sites stay compose-friendly.
     *
     * Replaces the `runCatching { requireHandle().…() }` shell that
     * appeared at every uniffi call site before. Two upsides:
     *
     *   1. One place to evolve handle access (e.g. swap in a future
     *      coroutine wrapper) without touching every method.
     *   2. The body of each repository method shrinks to the actual
     *      domain call + mappers, which makes the file readable end-to-end.
     *
     * The block runs synchronously on whatever thread Compose called from;
     * uniffi's generated bindings already serialise through the storage
     * core's mutex, so there's no thread-safety concern at this layer.
     */
    private inline fun <T> withStorage(block: OhdStorage.() -> T): Result<T> =
        runCatching { requireHandle().block() }

    /**
     * Path of the per-user `data.db` file. Lives inside the app's internal
     * storage so backup and restore are governed by the per-package backup
     * rules in `data_extraction_rules.xml`.
     */
    fun storageFile(): File {
        val ctx = requireNotNull(appContext) { "StorageRepository.init() not called" }
        return File(ctx.filesDir, "data.db")
    }

    fun isInitialised(): Boolean = storageFile().exists()

    /** True iff [openOrCreate] or [open] has populated a live handle in this process. */
    fun isOpen(): Boolean = handle != null

    /**
     * First-launch path. Calls `OhdStorage.create(path, keyHex)` and
     * issues a self-session token via `issueSelfSessionToken()`.
     *
     * `keyHex` should be a 64-char hex string (32 bytes). The v0 scaffold
     * uses a stub key derived from a stub passphrase — see the comment
     * inside `deriveStubKey` for the upgrade path.
     *
     * TODO: real key derivation per `spec/encryption.md`:
     *   - BIP39 phrase → seed → HKDF → K_file
     *   - K_file wraps K_envelope; K_envelope wraps per-blob keys
     *   - K_recovery is the user's printable backup
     *   For v0 we hardcode a single deployment-mode-A in-process key and
     *   document the whole flow as a TODO.
     */
    fun openOrCreate(keyHex: String): Result<Unit> = runCatching {
        val path = storageFile().absolutePath
        handle = OhdStorage.create(path, keyHex)
        Auth.recordStorageOpened(requireNotNull(appContext))
    }

    fun open(keyHex: String): Result<Unit> = runCatching {
        val path = storageFile().absolutePath
        handle = OhdStorage.open(path, keyHex)
        Auth.recordStorageOpened(requireNotNull(appContext))
    }

    /** Mint a fresh self-session token and persist it via [Auth]. */
    fun issueSelfSessionToken(): Result<String> = withStorage {
        issueSelfSessionToken().also {
            Auth.saveSelfSessionToken(requireNotNull(appContext), it)
        }
    }

    /** Snapshot of facts the Settings screen wants. */
    data class Identity(
        val storagePath: String,
        val userUlid: String,
        val tokenTruncated: String?,
        val formatVersion: String,
        val protocolVersion: String,
    )

    fun identity(): Identity {
        val ctx = requireNotNull(appContext)
        val token = Auth.getSelfSessionToken(ctx)
        val h = handle
        return Identity(
            storagePath = storageFile().absolutePath,
            userUlid = h?.userUlid() ?: "(storage not opened)",
            tokenTruncated = token?.let { it.take(10) + "…" + it.takeLast(4) },
            formatVersion = h?.formatVersion() ?: "(storage not opened)",
            protocolVersion = h?.protocolVersion() ?: "(storage not opened)",
        )
    }

    /**
     * Append a single event. The bindings collapse the OHDC `put_events`
     * batch RPC to one-at-a-time for ergonomic Kotlin call sites; bulk
     * imports from Health Connect will use a future `putEvents` overload.
     */
    fun putEvent(input: EventInput): Result<PutEventOutcome> = withStorage {
        putEvent(input.toDto()).toDomain()
    }

    /** Read recent events under self-session scope. */
    fun queryEvents(filter: EventFilter): Result<List<OhdEvent>> = withStorage {
        queryEvents(filter.toDto()).map { it.toDomain() }
    }

    /**
     * Pure SQL `COUNT(*)` over the same filter as [queryEvents]. Used by the
     * Home stat tile to side-step the 10 000-row response cap of `queryEvents`.
     * Channel-predicate / case-scope / grant filters are NOT applied — see
     * `core::events::count_events` in the Rust core for the contract.
     */
    fun countEvents(filter: EventFilter): Result<Long> = withStorage {
        countEvents(filter.toDto()).toLong()
    }

    /**
     * Soft-delete every event with `timestamp_ms < cutoffMs`. Used by the
     * free-tier 7-day retention worker. Returns the number of rows touched.
     */
    fun softDeleteEventsBefore(cutoffMs: Long): Result<Long> = withStorage {
        softDeleteEventsBefore(cutoffMs).toLong()
    }

    // =========================================================================
    // Agent tools (CORD + future MCP) — thin shim over ohd-mcp-core.
    // =========================================================================

    /** Tool catalog as JSON. Same payload the MCP server returns. */
    fun listToolsJson(): Result<String> = withStorage { listTools() }

    /** Execute one tool. JSON in, JSON out. Errors come back as `{"error": …}`. */
    fun executeToolJson(name: String, inputJson: String): Result<String> = withStorage {
        executeTool(name, inputJson)
    }

    // =========================================================================
    // Grants — Grants.{ListGrants,CreateGrant,RevokeGrant,UpdateGrant}
    // =========================================================================

    fun listGrants(includeRevoked: Boolean = false): Result<List<GrantSummary>> = withStorage {
        val filter = ListGrantsFilterDto(
            includeRevoked = includeRevoked,
            includeExpired = false,
            granteeKind = null,
            limit = null,
        )
        listGrants(filter).map { it.toDomain() }
    }

    fun createGrant(input: CreateGrantInput): Result<CreateGrantResult> = withStorage {
        createGrant(input.toDto()).toDomain()
    }

    fun revokeGrant(grantUlid: String, reason: String? = null): Result<Long> = withStorage {
        revokeGrant(grantUlid, reason)
        System.currentTimeMillis()
    }

    fun updateGrant(grantUlid: String, label: String?, expiresAtMs: Long?): Result<Unit> = withStorage {
        val update = GrantUpdateDto(
            granteeLabel = label,
            expiresAtMs = expiresAtMs,
        )
        updateGrant(grantUlid, update)
    }

    // =========================================================================
    // Pending — Pending.{ListPending,ApprovePending,RejectPending}
    // =========================================================================

    fun listPending(@Suppress("UNUSED_PARAMETER") status: String = "pending"):
            Result<List<PendingSummary>> = withStorage {
        // The uniffi `list_pending()` already filters to `pending` status by
        // default (see ohd-storage-bindings/src/lib.rs). The `status`
        // parameter is reserved for a future overload that takes a filter.
        listPending().map { it.toDomain() }
    }

    fun approvePending(pendingUlid: String, alsoTrustType: Boolean = false): Result<String> = withStorage {
        approvePending(pendingUlid, alsoTrustType)
        pendingUlid
    }

    fun rejectPending(pendingUlid: String, reason: String? = null): Result<Long> = withStorage {
        rejectPending(pendingUlid, reason)
        System.currentTimeMillis()
    }

    // =========================================================================
    // Cases — Cases.{ListCases,GetCase,ForceCloseCase,IssueRetrospectiveGrant}
    // =========================================================================

    fun listCases(includeClosed: Boolean = true): Result<List<CaseSummary>> = withStorage {
        // `null` state filter = open + closed; `Open` = open only.
        val filter: CaseStateDto? = if (includeClosed) null else CaseStateDto.OPEN
        listCases(filter).map { it.toDomain() }
    }

    fun getCase(caseUlid: String): Result<CaseDetail> = withStorage {
        getCase(caseUlid).toDomain()
    }

    fun forceCloseCase(
        caseUlid: String,
        @Suppress("UNUSED_PARAMETER") reason: String? = null,
    ): Result<Long> = withStorage {
        // The uniffi `force_close_case` doesn't yet take a reason; the v1
        // signature matches `close_case(case_id, None, false, None)`. Reason
        // is reserved for the v1.x API that adds operator-side closeout
        // notes.
        forceCloseCase(caseUlid)
        System.currentTimeMillis()
    }

    fun issueRetrospectiveGrant(caseUlid: String, input: CreateGrantInput): Result<CreateGrantResult> = withStorage {
        val req = RetroGrantInputDto(input = input.toDto())
        issueRetrospectiveGrant(caseUlid, req).toDomain()
    }

    // =========================================================================
    // Audit — Audit.AuditQuery
    // =========================================================================

    fun auditQuery(filter: AuditFilter): Result<List<AuditEntry>> = withStorage {
        // The uniffi filter accepts a single `action` string; the in-app
        // filter exposes a multi-select op_kinds list. We pick the first
        // for the first call and merge in-memory; if the list is empty we
        // pass null (= no action filter). For typical UI usage all three
        // kinds are selected so we pass null and merge nothing.
        val kindsForServer = if (filter.opKindsIn.size == 1) filter.opKindsIn.first() else null
        val dto = AuditFilterDto(
            fromMs = filter.fromMs,
            toMs = filter.toMs,
            actorType = null,
            action = kindsForServer,
            result = null,
            limit = filter.limit,
        )
        // Apply the multi-kind filter client-side because the uniffi
        // surface only carries one action at a time. (The OHDC wire RPC
        // accepts a list — pickup is a `actions_in: Vec<String>` field on
        // `AuditFilterDto`.)
        auditQuery(dto)
            .map { it.toDomain() }
            .filter { e -> filter.opKindsIn.isEmpty() || filter.opKindsIn.contains(e.opKind) }
            .filter { e -> filter.grantUlid == null || e.actorType == "grant" }
    }

    /** Recent export history — backs the ExportScreen "recent exports" list. */
    fun listExports(): Result<List<ExportRecord>> = runCatching {
        val ctx = requireNotNull(appContext)
        val outDir = File(ctx.filesDir, "exports")
        if (!outDir.exists()) return@runCatching emptyList<ExportRecord>()
        outDir.listFiles { f -> f.isFile }
            ?.sortedByDescending { it.lastModified() }
            ?.map { ExportRecord(it.absolutePath, it.lastModified(), it.length()) }
            ?: emptyList()
    }

    // =========================================================================
    // Emergency settings — Settings.{Get,Set}EmergencyConfig.
    //
    // The screen still keeps a local `EncryptedSharedPreferences` cache so
    // the UI is instant on cold start, but the source of truth is the
    // storage core (per `connect/spec/screens-emergency.md`). On every
    // write we mirror to both; on first read we promote whichever has
    // the more recent `updated_at_ms`.
    // =========================================================================

    fun getEmergencyConfig(): Result<EmergencyConfig> = runCatching {
        val ctx = requireNotNull(appContext)
        val local = EmergencyConfig.load(ctx)
        val h = handle
        if (h == null) return@runCatching local
        val remote = runCatching { h.getEmergencyConfig() }.getOrNull()
        if (remote == null) return@runCatching local
        // Storage's DTO becomes the truth as soon as the handle is open.
        // Map the FFI shape back into the in-app model and replace the
        // local cache so subsequent cold reads stay consistent.
        val merged = remote.toDomain(localFallback = local)
        merged.save(ctx)
        merged
    }

    fun setEmergencyConfig(cfg: EmergencyConfig): Result<Unit> = runCatching {
        val ctx = requireNotNull(appContext)
        cfg.save(ctx)
        // Mirror the write to storage when the handle is open.
        handle?.setEmergencyConfig(cfg.toDto())
    }

    // =========================================================================
    // Export — Export.{Export,GenerateDoctorPdf,MigrateInit,MigrateFinalize}
    // =========================================================================

    /** Returns the absolute path of the freshly-written .ohd portable file. */
    fun exportAll(): Result<String> = runCatching {
        val ctx = requireNotNull(appContext)
        val bytes = requireHandle().exportAll()
        val outDir = File(ctx.filesDir, "exports").apply { mkdirs() }
        val target = File(outDir, "ohd-export-${System.currentTimeMillis()}.ohd")
        target.writeBytes(bytes)
        target.absolutePath
    }

    fun generateDoctorPdf(): Result<String> = runCatching {
        // TODO: requires uniffi binding — handle.generateDoctorPdf() once
        //       storage ships Export.GenerateDoctorPdf. Until then we render a
        //       one-page summary client-side via Android's PdfDocument API in
        //       ExportScreen.kt (which does the actual rendering).
        throw UnsupportedOperationException("Doctor PDF: rendered client-side; see ExportScreen")
    }
}

// =============================================================================
// In-app FFI-shaped DTOs (mirror the uniffi types so callers can build them
// without importing `uniffi.ohd_storage.*` directly)
// =============================================================================

/** Sparse event input. Maps 1:1 onto `EventInputDto` from the uniffi surface. */
data class EventInput(
    val timestampMs: Long,
    val durationMs: Long? = null,
    val tzOffsetMinutes: Int? = null,
    val tzName: String? = null,
    val eventType: String,
    val channels: List<EventChannelInput>,
    val deviceId: String? = null,
    val appName: String? = "OHD Connect Android",
    val appVersion: String? = null,
    val source: String? = "manual:android_app",
    val sourceId: String? = null,
    val notes: String? = null,
    /**
     * `false` for detail rows the UI groups under a parent (`intake.*` under
     * a `food.eaten`, `measurement.ecg_second` under an `ecg_session`).
     * Defaults to `true` so every existing call site keeps minting entry-level
     * events. Producers grouping a set of detail rows emit a `correlation_id`
     * channel to thread them together.
     */
    val topLevel: Boolean = true,
)

data class EventChannelInput(
    val path: String,
    val scalar: OhdScalar,
)

/** Subset of the OHDC `EventFilter`. Mirrors `EventFilterDto`. */
data class EventFilter(
    val fromMs: Long? = null,
    val toMs: Long? = null,
    val eventTypesIn: List<String> = emptyList(),
    val eventTypesNotIn: List<String> = emptyList(),
    val includeDeleted: Boolean = false,
    val limit: Long? = 50,
    /**
     * Default `All` matches the historical behaviour. UI surfaces that want
     * to hide derived rows (Recent, History list, home event count) pass
     * `TopLevelOnly`. Drill-down ("show me the children of this row") uses
     * `NonTopLevelOnly` together with a `correlation_id` channel filter.
     */
    val visibility: EventVisibility = EventVisibility.All,
    /**
     * Restrict to events whose `source` matches one of these strings exactly.
     * Used by the Health Connect sync watermark: looks up the latest event
     * of a given type from `source = "health_connect"` to compute the
     * incremental cursor.
     */
    val sourceIn: List<String> = emptyList(),
)

/** Filter dimension over the `top_level` column. */
enum class EventVisibility(internal val wire: String?) {
    All(null),
    TopLevelOnly("top_level_only"),
    NonTopLevelOnly("non_top_level_only");
}

// =============================================================================
// Grants / Pending / Cases / Audit DTOs
//
// These mirror the protobuf shapes from
// `storage/proto/ohdc/v0/ohdc.proto` flattened to in-app types so the Compose
// layer doesn't import generated proto / uniffi types directly.
// =============================================================================

/** Mirrors a small subset of the OHDC `Grant` message — what the list view shows. */
data class GrantSummary(
    val ulid: String,
    val granteeLabel: String,
    val granteeKind: String,
    val purpose: String?,
    val createdAtMs: Long,
    val expiresAtMs: Long?,
    val revokedAtMs: Long?,
    val approvalMode: String,
    val defaultAction: String,
    val readEventTypes: List<String>,
    val writeEventTypes: List<String>,
    val deniedSensitivityClasses: List<String>,
    val lastUsedMs: Long?,
    val useCount: Long,
)

/** Template-shaped input for `Grants.CreateGrant`. */
data class CreateGrantInput(
    val granteeLabel: String,
    val granteeKind: String,            // 'user' | 'role' | 'org' | 'researcher' | 'emergency_authority' | 'delegate'
    val purpose: String? = null,
    val approvalMode: String,           // 'always' | 'auto_for_event_types' | 'never_required'
    val defaultAction: String,          // 'allow' | 'deny'
    val expiresAtMs: Long? = null,
    val readEventTypes: List<String> = emptyList(),
    val writeEventTypes: List<String> = emptyList(),
    val autoApproveEventTypes: List<String> = emptyList(),
    val denySensitivityClasses: List<String> = emptyList(),
    val notifyOnAccess: Boolean = false,
    val stripNotes: Boolean = false,
    val aggregationOnly: Boolean = false,
)

data class CreateGrantResult(
    val grantUlid: String,
    val token: String,
    val shareUrl: String,
)

/** One row in the Pending tab. Mirrors the OHDC `PendingEvent`. */
data class PendingSummary(
    val ulid: String,
    val submittingGrantUlid: String,
    val submittingGrantLabel: String,
    val eventType: String,
    val keyChannelDisplay: String,      // e.g. "glucose: 6.4 mmol/L"
    val submittedAtMs: Long,
    val status: String,                 // 'pending' | 'approved' | 'rejected' | 'expired'
)

/** One row in the Cases tab. Mirrors the OHDC `Case`. */
data class CaseSummary(
    val ulid: String,
    val caseType: String,
    val label: String?,
    val startedAtMs: Long,
    val endedAtMs: Long?,
    val authorityLabel: String?,
    val authorityGrantUlid: String?,
    val autoGranted: Boolean,
)

data class CaseDetail(
    val ulid: String,
    val caseType: String,
    val label: String?,
    val startedAtMs: Long,
    val endedAtMs: Long?,
    val authorityLabel: String?,
    val autoGranted: Boolean,
    val timeline: List<OhdEvent>,
    val audit: List<AuditEntry>,
    val handoffChain: List<HandoffEntry>,
)

data class HandoffEntry(
    val authorityLabel: String,
    val tsMs: Long,
    val toAuthority: String?,
)

/** Audit query filter. Mirrors `AuditQueryRequest` from the proto. */
data class AuditFilter(
    val grantUlid: String? = null,
    val opKindsIn: List<String> = emptyList(),  // 'read' | 'write' | 'grant_mgmt'
    val fromMs: Long? = null,
    val toMs: Long? = null,
    val limit: Long? = 200,
)

data class AuditEntry(
    val ulid: String,
    val tsMs: Long,
    val actorType: String,              // 'self' | 'grant' | 'delegate'
    val actorLabel: String,
    val opKind: String,                 // 'read' | 'write' | 'grant_mgmt' | 'sync'
    val opName: String,                 // RPC method name, e.g. 'QueryEvents'
    val querySummary: String?,
    val rowsReturned: Long?,
    val rowsFiltered: Long?,
    val autoGranted: Boolean,           // emergency timeout-default-allow
)

/** Recent-export row backed by `<filesDir>/exports/`. */
data class ExportRecord(
    val absolutePath: String,
    val createdAtMs: Long,
    val sizeBytes: Long,
)

// =============================================================================
// Mappers — in-app types ↔ uniffi DTOs
//
// File-private extension functions; the Compose layer never sees a
// `uniffi.ohd_storage.*` symbol because every call goes through one of
// these.
// =============================================================================

internal fun OhdScalar.toDto(channelPath: String): ChannelValueDto = when (this) {
    is OhdScalar.Real -> ChannelValueDto(
        channelPath = channelPath,
        valueKind = ValueKind.REAL,
        realValue = v,
        intValue = null,
        boolValue = null,
        textValue = null,
        enumOrdinal = null,
    )
    is OhdScalar.Int -> ChannelValueDto(
        channelPath = channelPath,
        valueKind = ValueKind.INT,
        realValue = null,
        intValue = v,
        boolValue = null,
        textValue = null,
        enumOrdinal = null,
    )
    is OhdScalar.Bool -> ChannelValueDto(
        channelPath = channelPath,
        valueKind = ValueKind.BOOL,
        realValue = null,
        intValue = null,
        boolValue = v,
        textValue = null,
        enumOrdinal = null,
    )
    is OhdScalar.Text -> ChannelValueDto(
        channelPath = channelPath,
        valueKind = ValueKind.TEXT,
        realValue = null,
        intValue = null,
        boolValue = null,
        textValue = v,
        enumOrdinal = null,
    )
    is OhdScalar.EnumOrdinal -> ChannelValueDto(
        channelPath = channelPath,
        valueKind = ValueKind.ENUM_ORDINAL,
        realValue = null,
        intValue = null,
        boolValue = null,
        textValue = null,
        enumOrdinal = ordinal,
    )
}

internal fun ChannelValueDto.toDomain(): OhdChannel {
    val scalar: OhdScalar = when (valueKind) {
        ValueKind.REAL -> OhdScalar.Real(realValue ?: 0.0)
        ValueKind.INT -> OhdScalar.Int(intValue ?: 0L)
        ValueKind.BOOL -> OhdScalar.Bool(boolValue ?: false)
        ValueKind.TEXT -> OhdScalar.Text(textValue ?: "")
        ValueKind.ENUM_ORDINAL -> OhdScalar.EnumOrdinal(enumOrdinal ?: 0)
    }
    val display: String = when (scalar) {
        is OhdScalar.Real -> scalar.v.toString()
        is OhdScalar.Int -> scalar.v.toString()
        is OhdScalar.Bool -> scalar.v.toString()
        is OhdScalar.Text -> scalar.v
        is OhdScalar.EnumOrdinal -> "enum:${scalar.ordinal}"
    }
    return OhdChannel(path = channelPath, display = display, scalar = scalar)
}

internal fun EventInput.toDto(): EventInputDto = EventInputDto(
    timestampMs = timestampMs,
    durationMs = durationMs,
    tzOffsetMinutes = tzOffsetMinutes,
    tzName = tzName,
    eventType = eventType,
    channels = channels.map { it.scalar.toDto(it.path) },
    deviceId = deviceId,
    appName = appName,
    appVersion = appVersion,
    source = source,
    sourceId = sourceId,
    notes = notes,
    topLevel = topLevel,
)

internal fun EventFilter.toDto(): EventFilterDto = EventFilterDto(
    fromMs = fromMs,
    toMs = toMs,
    eventTypesIn = eventTypesIn,
    eventTypesNotIn = eventTypesNotIn,
    includeDeleted = includeDeleted,
    limit = limit,
    visibility = visibility.wire,
    sourceIn = sourceIn,
)

internal fun EventDto.toDomain(): OhdEvent = OhdEvent(
    ulid = ulid,
    timestampMs = timestampMs,
    durationMs = durationMs,
    eventType = eventType,
    channels = channels.map { it.toDomain() },
    notes = notes,
    source = source,
    topLevel = topLevel,
)

internal fun PutEventOutcomeDto.toDomain(): PutEventOutcome = when (outcome) {
    "committed" -> PutEventOutcome.Committed(ulid = ulid, timestampMs = timestampMs)
    "pending" -> PutEventOutcome.Pending(ulid = ulid, expiresAtMs = timestampMs)
    else -> PutEventOutcome.Error(code = errorCode, message = errorMessage)
}

internal fun GrantDto.toDomain(): GrantSummary {
    val readEventTypes = eventTypeRules
        .filter { it.effect == "allow" }
        .map { it.eventType }
    val writeEventTypes = emptyList<String>() // uniffi GrantDto doesn't surface write-rules separately yet
    val deniedSensitivities = sensitivityRules
        .filter { it.effect == "deny" }
        .map { it.sensitivityClass }
    return GrantSummary(
        ulid = ulid,
        granteeLabel = granteeLabel,
        granteeKind = granteeKind,
        purpose = purpose,
        createdAtMs = createdAtMs,
        expiresAtMs = expiresAtMs,
        revokedAtMs = revokedAtMs,
        approvalMode = approvalMode,
        defaultAction = defaultAction,
        readEventTypes = readEventTypes,
        writeEventTypes = writeEventTypes,
        deniedSensitivityClasses = deniedSensitivities,
        lastUsedMs = null,                     // not on uniffi GrantDto yet
        useCount = 0L,                         // not on uniffi GrantDto yet
    )
}

internal fun CreateGrantInput.toDto(): CreateGrantInputDto = CreateGrantInputDto(
    granteeLabel = granteeLabel,
    granteeKind = granteeKind,
    purpose = purpose,
    defaultAction = defaultAction,
    approvalMode = approvalMode,
    expiresAtMs = expiresAtMs,
    eventTypeRules = readEventTypes.map {
        GrantEventTypeRuleDto(eventType = it, effect = "allow")
    },
    channelRules = emptyList(),
    sensitivityRules = denySensitivityClasses.map {
        GrantSensitivityRuleDto(sensitivityClass = it, effect = "deny")
    },
    writeEventTypeRules = writeEventTypes.map {
        GrantEventTypeRuleDto(eventType = it, effect = "allow")
    },
    autoApproveEventTypes = autoApproveEventTypes,
    aggregationOnly = aggregationOnly,
    stripNotes = stripNotes,
    notifyOnAccess = notifyOnAccess,
)

internal fun GrantTokenDto.toDomain(): CreateGrantResult = CreateGrantResult(
    grantUlid = grantUlid,
    token = token,
    shareUrl = shareUrl,
)

internal fun PendingEventDto.toDomain(): PendingSummary {
    val keyChannel = event.channels.firstOrNull()
    val display = keyChannel?.let {
        val s = it.toDomain()
        "${it.channelPath}: ${s.display}"
    } ?: "(no channels)"
    return PendingSummary(
        ulid = ulid,
        submittingGrantUlid = submittingGrantUlid ?: "(unknown)",
        submittingGrantLabel = submittingGrantUlid?.take(8)?.let { "$it…" } ?: "(unknown grant)",
        eventType = event.eventType,
        keyChannelDisplay = display,
        submittedAtMs = submittedAtMs,
        status = status,
    )
}

internal fun CaseDto.toDomain(): CaseSummary = CaseSummary(
    ulid = ulid,
    caseType = caseType,
    label = caseLabel,
    startedAtMs = startedAtMs,
    endedAtMs = endedAtMs,
    authorityLabel = openingAuthorityGrantUlid?.let { "Authority ${it.take(8)}…" },
    authorityGrantUlid = openingAuthorityGrantUlid,
    // The uniffi `CaseDto` doesn't carry a separate `auto_granted` flag —
    // the field is implied by the case_type ("emergency") + the timeout
    // policy. Surface as `true` for emergency cases so the UI badge fires.
    autoGranted = caseType == "emergency",
)

internal fun CaseDetailDto.toDomain(): CaseDetail {
    val summary = case.toDomain()
    return CaseDetail(
        ulid = summary.ulid,
        caseType = summary.caseType,
        label = summary.label,
        startedAtMs = summary.startedAtMs,
        endedAtMs = summary.endedAtMs,
        authorityLabel = summary.authorityLabel,
        autoGranted = summary.autoGranted,
        timeline = emptyList(), // CaseDetail timeline is reserved for the future RPC; v0 audit is enough
        audit = audit.map { it.toDomain() },
        handoffChain = case.predecessorCaseUlid
            ?.let {
                listOf(
                    HandoffEntry(
                        authorityLabel = "predecessor ${it.take(8)}…",
                        tsMs = summary.startedAtMs,
                        toAuthority = summary.authorityLabel,
                    ),
                )
            }
            ?: emptyList(),
    )
}

internal fun AuditEntryDto.toDomain(): AuditEntry {
    // Action strings from storage carry the OHDC RPC tag; map a few that
    // the UI groups by op-kind, fall through to `read` so unknown actions
    // still show up in the list.
    val opKind = when (action) {
        "PutEvents", "ApprovePending", "RejectPending" -> "write"
        "CreateGrant", "RevokeGrant", "UpdateGrant" -> "grant_mgmt"
        else -> "read"
    }
    return AuditEntry(
        ulid = "audit-${tsMs}-${action}",  // synthetic; uniffi AuditEntryDto has no ulid
        tsMs = tsMs,
        actorType = actorType,
        actorLabel = if (actorType == "self") "self" else actorType,
        opKind = opKind,
        opName = action,
        querySummary = queryParamsJson,
        rowsReturned = rowsReturned,
        rowsFiltered = rowsFiltered,
        autoGranted = result == "auto_granted",
    )
}

internal fun EmergencyConfig.toDto(): EmergencyConfigDto = EmergencyConfigDto(
    enabled = featureEnabled,
    bluetoothBeacon = bleBeacon,
    approvalTimeoutSeconds = approvalTimeoutSeconds,
    defaultActionOnTimeout = when (defaultOnTimeout) {
        EmergencyConfig.DefaultAction.ALLOW -> "allow"
        EmergencyConfig.DefaultAction.REFUSE -> "refuse"
    },
    lockScreenVisibility = when (lockScreenMode) {
        EmergencyConfig.LockScreenMode.FULL -> "full"
        EmergencyConfig.LockScreenMode.BASIC_ONLY -> "basic_only"
    },
    historyWindowHours = historyWindowHours,
    channelPathsAllowed = buildList {
        if (channels.glucose) add("std.blood_glucose.value")
        if (channels.heartRate) add("std.heart_rate_resting.value")
        if (channels.bloodPressure) add("std.blood_pressure.systolic")
        if (channels.bloodPressure) add("std.blood_pressure.diastolic")
        if (channels.spo2) add("std.spo2.value")
        if (channels.temperature) add("std.body_temperature.value")
        if (channels.allergies) add("std.allergy.value")
        if (channels.medications) add("std.medication_dose.value")
        if (channels.bloodType) add("std.blood_type.value")
        if (channels.advanceDirectives) add("std.advance_directive.value")
        if (channels.diagnoses) add("std.diagnosis.value")
    },
    sensitivityClassesAllowed = buildList {
        if (sensitivity.general) add("general")
        if (sensitivity.mentalHealth) add("mental_health")
        if (sensitivity.substanceUse) add("substance_use")
        if (sensitivity.sexualHealth) add("sexual_health")
        if (sensitivity.reproductive) add("reproductive")
    },
    shareLocation = locationShare,
    trustedAuthorities = trustRoots.map {
        TrustedAuthorityDto(
            label = it.name,
            scope = it.scope,
            publicKeyPem = null,                  // local-only roots; v0.x adds cert paste
            isDefault = !it.removable,
        )
    },
    bystanderProxyEnabled = bystanderProxy,
    updatedAtMs = System.currentTimeMillis(),
)

/**
 * Map a server-side EmergencyConfigDto back to the in-app `EmergencyConfig`.
 * Falls back to the local cache for any field the server doesn't surface
 * (the channel/sensitivity flag breakdown is denormalised on the wire as
 * a list of channel paths, so we re-derive flags by membership).
 */
internal fun EmergencyConfigDto.toDomain(localFallback: EmergencyConfig): EmergencyConfig {
    fun has(path: String) = channelPathsAllowed.any { it.startsWith(path) }
    val ch = EmergencyConfig.ChannelToggles(
        glucose = has("std.blood_glucose"),
        heartRate = has("std.heart_rate_resting"),
        bloodPressure = has("std.blood_pressure"),
        spo2 = has("std.spo2"),
        temperature = has("std.body_temperature"),
        allergies = has("std.allergy"),
        medications = has("std.medication_dose"),
        bloodType = has("std.blood_type"),
        advanceDirectives = has("std.advance_directive"),
        diagnoses = has("std.diagnosis"),
    )
    val s = EmergencyConfig.SensitivityToggles(
        general = sensitivityClassesAllowed.contains("general"),
        mentalHealth = sensitivityClassesAllowed.contains("mental_health"),
        substanceUse = sensitivityClassesAllowed.contains("substance_use"),
        sexualHealth = sensitivityClassesAllowed.contains("sexual_health"),
        reproductive = sensitivityClassesAllowed.contains("reproductive"),
    )
    val roots = trustedAuthorities.mapIndexed { idx, t ->
        EmergencyConfig.TrustRoot(
            id = if (t.isDefault) "ohd_default" else "remote_$idx",
            name = t.label,
            scope = t.scope ?: "global",
            removable = !t.isDefault,
        )
    }.ifEmpty { localFallback.trustRoots }
    return EmergencyConfig(
        featureEnabled = enabled,
        bleBeacon = bluetoothBeacon,
        approvalTimeoutSeconds = approvalTimeoutSeconds,
        defaultOnTimeout = when (defaultActionOnTimeout) {
            "refuse" -> EmergencyConfig.DefaultAction.REFUSE
            else -> EmergencyConfig.DefaultAction.ALLOW
        },
        lockScreenMode = when (lockScreenVisibility) {
            "basic_only" -> EmergencyConfig.LockScreenMode.BASIC_ONLY
            else -> EmergencyConfig.LockScreenMode.FULL
        },
        historyWindowHours = historyWindowHours,
        channels = ch,
        sensitivity = s,
        locationShare = shareLocation,
        bystanderProxy = bystanderProxyEnabled,
        trustRoots = roots,
    )
}
