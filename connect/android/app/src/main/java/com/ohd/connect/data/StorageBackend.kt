package com.ohd.connect.data

import uniffi.ohd_storage.RemoteShareDto
import uniffi.ohd_storage.ShareResponderHandle

/**
 * The storage-operation seam.
 *
 * Every data operation `StorageRepository` exposes is declared here with the
 * exact same signature and return type the public `StorageRepository` method
 * carries today. `StorageRepository` stays an `object` singleton and a stable
 * facade for its ~39 call sites; internally it now delegates each operation to
 * an [activeBackend][StorageRepository] implementation of this interface.
 *
 * Phase 0 ships a single implementation, [LocalStorageBackend], which wraps the
 * local `OhdStorage` uniffi handle — the runtime path is byte-for-byte
 * equivalent to the pre-refactor code.
 *
 * Phase 3 adds a `RemoteStorageBackend` (an OHDC client over the network). The
 * open-time selection in `StorageRepository.openOrCreate` / `open` is the one
 * place that decides which implementation backs the singleton; no call site of
 * `StorageRepository` changes.
 *
 * Lifecycle (`init`, `open`, `openOrCreate`, `isOpen`, `isInitialised`,
 * `storageFile`, `identity`, `storageIdentityKey`, `activeMode`) is NOT part of
 * this interface — it stays on `StorageRepository`, which owns backend
 * construction and selection.
 */
interface StorageBackend {

    // --- Events --------------------------------------------------------------

    fun putEvent(input: EventInput): Result<PutEventOutcome>

    /**
     * Append a batch of events in one call. Against remote storage this is a
     * single `PutEvents` RPC instead of one round-trip per event; `atomic`
     * asks the server to commit all-or-nothing. Returns one outcome per input,
     * in order.
     */
    fun putEvents(inputs: List<EventInput>, atomic: Boolean): Result<List<PutEventOutcome>>

    fun queryEvents(filter: EventFilter): Result<List<OhdEvent>>

    fun countEvents(filter: EventFilter): Result<Long>

    /**
     * Distinct `source` count within `filter`. `SELECT COUNT(DISTINCT source)`
     * on either backend. Drives the home-screen "sources" tile.
     */
    fun countSources(filter: EventFilter): Result<Long>

    /**
     * Distinct event-type names + per-type counts within `filter`, sorted
     * count-DESC. One SQL `GROUP BY` (or its remote-RPC equivalent). Used
     * by the History chip set.
     */
    fun listEventTypes(filter: EventFilter): Result<List<EventTypeSummary>>

    fun softDeleteEventsBefore(cutoffMs: Long): Result<Long>

    /**
     * Hard-delete every event with `timestamp_ms` in the inclusive range
     * `[fromMs, toMs]`. Optionally filter by `eventTypes`. Returns the
     * number of rows removed. Owner-only — the food-log row delete on
     * [com.ohd.connect.ui.screens.FoodScreen] routes through here.
     */
    fun hardDeleteEventsInRange(
        fromMs: Long,
        toMs: Long,
        eventTypes: List<String>,
    ): Result<Long>

    // --- Agent tools ---------------------------------------------------------

    fun listToolsJson(): Result<String>

    fun executeToolJson(name: String, inputJson: String): Result<String>

    // --- Remote access (CORD data-link share responder) ----------------------

    fun registerRemoteShare(
        grantUlid: String,
        relayOrigin: String,
        identityKeyHex: String,
        shareLabel: String?,
    ): Result<RemoteShareDto>

    fun startShareResponder(
        grantUlid: String,
        share: RemoteShareDto,
        relayTunnelUrl: String,
        identityKeyHex: String,
        allowInsecureDev: Boolean,
    ): Result<ShareResponderHandle>

    // --- Grants --------------------------------------------------------------

    fun listGrants(includeRevoked: Boolean): Result<List<GrantSummary>>

    fun createGrant(input: CreateGrantInput): Result<CreateGrantResult>

    fun revokeGrant(grantUlid: String, reason: String?): Result<Long>

    fun updateGrant(grantUlid: String, label: String?, expiresAtMs: Long?): Result<Unit>

    fun setGrantSuspended(grantUlid: String, suspended: Boolean): Result<Unit>

    fun getGrant(grantUlid: String): Result<GrantSummary?>

    // --- Pending -------------------------------------------------------------

    fun listPending(status: String): Result<List<PendingSummary>>

    fun approvePending(pendingUlid: String, alsoTrustType: Boolean): Result<String>

    fun rejectPending(pendingUlid: String, reason: String?): Result<Long>

    // --- Cases ---------------------------------------------------------------

    fun listCases(includeClosed: Boolean): Result<List<CaseSummary>>

    fun getCase(caseUlid: String): Result<CaseDetail>

    fun forceCloseCase(caseUlid: String, reason: String?): Result<Long>

    fun issueRetrospectiveGrant(caseUlid: String, input: CreateGrantInput): Result<CreateGrantResult>

    // --- Audit ---------------------------------------------------------------

    fun auditQuery(filter: AuditFilter): Result<List<AuditEntry>>

    // --- Self-session token --------------------------------------------------

    fun issueSelfSessionToken(): Result<String>

    // --- Export --------------------------------------------------------------

    /** Raw `.ohd` portable bytes. `StorageRepository.exportAll` writes them to disk. */
    fun exportAll(): Result<ByteArray>

    fun generateDoctorPdf(): Result<String>

    // --- Emergency config ----------------------------------------------------

    /**
     * Server-side emergency config, or `null` when the backend can't supply
     * one (e.g. handle not open). `StorageRepository.getEmergencyConfig`
     * reconciles this against the local `EncryptedSharedPreferences` cache.
     */
    fun getEmergencyConfig(): EmergencyConfigSnapshot?

    /** Mirror an emergency-config write to the backend. No-op when unavailable. */
    fun setEmergencyConfig(cfg: EmergencyConfig)

    // --- Identity facts ------------------------------------------------------

    /** `user_ulid()` or `null` when the backend is not open. */
    fun userUlidOrNull(): String?

    /** `format_version()` or `null` when the backend is not open. */
    fun formatVersionOrNull(): String?

    /** `protocol_version()` or `null` when the backend is not open. */
    fun protocolVersionOrNull(): String?
}

/**
 * Carrier for a backend-supplied emergency config. Wraps the FFI
 * `EmergencyConfigDto` so the [StorageBackend] surface does not leak a
 * `uniffi.ohd_storage.*` type, while keeping the existing
 * `EmergencyConfigDto.toDomain(localFallback)` reconciliation in
 * `StorageRepository`.
 */
class EmergencyConfigSnapshot(
    internal val dto: uniffi.ohd_storage.EmergencyConfigDto,
)
