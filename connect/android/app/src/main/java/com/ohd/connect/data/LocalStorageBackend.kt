package com.ohd.connect.data

import uniffi.ohd_storage.AuditFilterDto
import uniffi.ohd_storage.CaseStateDto
import uniffi.ohd_storage.GrantUpdateDto
import uniffi.ohd_storage.ListGrantsFilterDto
import uniffi.ohd_storage.OhdStorage
import uniffi.ohd_storage.RemoteShareDto
import uniffi.ohd_storage.RetroGrantInputDto
import uniffi.ohd_storage.ShareResponderHandle

/**
 * [StorageBackend] implementation that wraps the local `OhdStorage` uniffi
 * handle — the Rust storage core running in-process.
 *
 * This is the `OnDevice` backend and, for Phase 0, the only one. Each method
 * body is the verbatim per-method logic that used to live inline in
 * `StorageRepository` (the `withStorage { handle.… }` pattern). The runtime
 * path is byte-for-byte equivalent to the pre-refactor code.
 *
 * The owned [handle] is the live uniffi `OhdStorage`. `StorageRepository`
 * constructs one of these in `openOrCreate` / `open` once the user has
 * finished the Setup screen, and tears it down on `close`.
 *
 * Thread-safety: uniffi's generated bindings already serialise through the
 * storage core's mutex, so calls are made synchronously on whatever thread
 * Compose called from — no extra locking at this layer.
 */
internal class LocalStorageBackend(
    private val handle: OhdStorage,
) : StorageBackend {

    /**
     * Run `block` against the uniffi handle, wrapping the result in a [Result]
     * so call sites stay compose-friendly. Mirrors the `withStorage` shell
     * `StorageRepository` used before the refactor.
     */
    private inline fun <T> withStorage(block: OhdStorage.() -> T): Result<T> =
        runCatching { handle.block() }

    // --- Events --------------------------------------------------------------

    override fun putEvent(input: EventInput): Result<PutEventOutcome> = withStorage {
        putEvent(input.toDto()).toDomain()
    }

    override fun putEvents(
        inputs: List<EventInput>,
        atomic: Boolean,
    ): Result<List<PutEventOutcome>> = withStorage {
        putEvents(inputs.map { it.toDto() }, atomic).map { it.toDomain() }
    }

    override fun queryEvents(filter: EventFilter): Result<List<OhdEvent>> = withStorage {
        queryEvents(filter.toDto()).map { it.toDomain() }
    }

    override fun listEventTypes(filter: EventFilter): Result<List<EventTypeSummary>> = withStorage {
        listEventTypes(filter.toDto()).map { EventTypeSummary(it.eventType, it.count) }
    }

    override fun countEvents(filter: EventFilter): Result<Long> = withStorage {
        countEvents(filter.toDto()).toLong()
    }

    override fun softDeleteEventsBefore(cutoffMs: Long): Result<Long> = withStorage {
        softDeleteEventsBefore(cutoffMs).toLong()
    }

    // --- Agent tools ---------------------------------------------------------

    override fun listToolsJson(): Result<String> = withStorage { listTools() }

    override fun executeToolJson(name: String, inputJson: String): Result<String> = withStorage {
        executeTool(name, inputJson)
    }

    // --- Remote access -------------------------------------------------------

    override fun registerRemoteShare(
        grantUlid: String,
        relayOrigin: String,
        identityKeyHex: String,
        shareLabel: String?,
    ): Result<RemoteShareDto> = withStorage {
        registerRemoteShare(grantUlid, relayOrigin, identityKeyHex, shareLabel)
    }

    override fun startShareResponder(
        grantUlid: String,
        share: RemoteShareDto,
        relayTunnelUrl: String,
        identityKeyHex: String,
        allowInsecureDev: Boolean,
    ): Result<ShareResponderHandle> = withStorage {
        startShareResponder(grantUlid, share, relayTunnelUrl, identityKeyHex, allowInsecureDev)
    }

    // --- Grants --------------------------------------------------------------

    override fun listGrants(includeRevoked: Boolean): Result<List<GrantSummary>> = withStorage {
        val filter = ListGrantsFilterDto(
            includeRevoked = includeRevoked,
            includeExpired = false,
            granteeKind = null,
            limit = null,
        )
        listGrants(filter).map { it.toDomain() }
    }

    override fun createGrant(input: CreateGrantInput): Result<CreateGrantResult> = withStorage {
        createGrant(input.toDto()).toDomain()
    }

    override fun revokeGrant(grantUlid: String, reason: String?): Result<Long> = withStorage {
        revokeGrant(grantUlid, reason)
        System.currentTimeMillis()
    }

    override fun updateGrant(grantUlid: String, label: String?, expiresAtMs: Long?): Result<Unit> = withStorage {
        val update = GrantUpdateDto(
            granteeLabel = label,
            expiresAtMs = expiresAtMs,
        )
        updateGrant(grantUlid, update)
    }

    override fun setGrantSuspended(grantUlid: String, suspended: Boolean): Result<Unit> = withStorage {
        setGrantSuspended(grantUlid, suspended)
    }

    override fun getGrant(grantUlid: String): Result<GrantSummary?> = withStorage {
        val filter = ListGrantsFilterDto(
            includeRevoked = true,
            includeExpired = true,
            granteeKind = null,
            limit = null,
        )
        listGrants(filter).map { it.toDomain() }.firstOrNull { it.ulid == grantUlid }
    }

    // --- Pending -------------------------------------------------------------

    override fun listPending(
        @Suppress("UNUSED_PARAMETER") status: String,
    ): Result<List<PendingSummary>> = withStorage {
        // The uniffi `list_pending()` already filters to `pending` status by
        // default (see ohd-storage-bindings/src/lib.rs). The `status`
        // parameter is reserved for a future overload that takes a filter.
        listPending().map { it.toDomain() }
    }

    override fun approvePending(pendingUlid: String, alsoTrustType: Boolean): Result<String> = withStorage {
        approvePending(pendingUlid, alsoTrustType)
        pendingUlid
    }

    override fun rejectPending(pendingUlid: String, reason: String?): Result<Long> = withStorage {
        rejectPending(pendingUlid, reason)
        System.currentTimeMillis()
    }

    // --- Cases ---------------------------------------------------------------

    override fun listCases(includeClosed: Boolean): Result<List<CaseSummary>> = withStorage {
        // `null` state filter = open + closed; `Open` = open only.
        val filter: CaseStateDto? = if (includeClosed) null else CaseStateDto.OPEN
        listCases(filter).map { it.toDomain() }
    }

    override fun getCase(caseUlid: String): Result<CaseDetail> = withStorage {
        getCase(caseUlid).toDomain()
    }

    override fun forceCloseCase(
        caseUlid: String,
        @Suppress("UNUSED_PARAMETER") reason: String?,
    ): Result<Long> = withStorage {
        // The uniffi `force_close_case` doesn't yet take a reason; the v1
        // signature matches `close_case(case_id, None, false, None)`. Reason
        // is reserved for the v1.x API that adds operator-side closeout
        // notes.
        forceCloseCase(caseUlid)
        System.currentTimeMillis()
    }

    override fun issueRetrospectiveGrant(
        caseUlid: String,
        input: CreateGrantInput,
    ): Result<CreateGrantResult> = withStorage {
        val req = RetroGrantInputDto(input = input.toDto())
        issueRetrospectiveGrant(caseUlid, req).toDomain()
    }

    // --- Audit ---------------------------------------------------------------

    override fun auditQuery(filter: AuditFilter): Result<List<AuditEntry>> = withStorage {
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

    // --- Self-session token --------------------------------------------------

    override fun issueSelfSessionToken(): Result<String> = withStorage {
        issueSelfSessionToken()
    }

    // --- Export --------------------------------------------------------------

    override fun exportAll(): Result<ByteArray> = withStorage {
        exportAll()
    }

    override fun generateDoctorPdf(): Result<String> = runCatching {
        // TODO: requires uniffi binding — handle.generateDoctorPdf() once
        //       storage ships Export.GenerateDoctorPdf. Until then we render a
        //       one-page summary client-side via Android's PdfDocument API in
        //       ExportScreen.kt (which does the actual rendering).
        throw UnsupportedOperationException("Doctor PDF: rendered client-side; see ExportScreen")
    }

    // --- Emergency config ----------------------------------------------------

    override fun getEmergencyConfig(): EmergencyConfigSnapshot? =
        runCatching { handle.getEmergencyConfig() }.getOrNull()
            ?.let { EmergencyConfigSnapshot(it) }

    override fun setEmergencyConfig(cfg: EmergencyConfig) {
        handle.setEmergencyConfig(cfg.toDto())
    }

    // --- Identity facts ------------------------------------------------------

    override fun userUlidOrNull(): String? = handle.userUlid()

    override fun formatVersionOrNull(): String? = handle.formatVersion()

    override fun protocolVersionOrNull(): String? = handle.protocolVersion()
}
