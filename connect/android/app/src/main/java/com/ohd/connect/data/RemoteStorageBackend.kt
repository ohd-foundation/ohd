package com.ohd.connect.data

import android.content.Context
import android.os.Looper
import android.util.Log
import uniffi.ohd_storage.AuditFilterDto
import uniffi.ohd_storage.CaseStateDto
import uniffi.ohd_storage.ListGrantsFilterDto
import uniffi.ohd_storage.OhdException
import uniffi.ohd_storage.RemoteOhdStorage
import uniffi.ohd_storage.RemoteShareDto
import uniffi.ohd_storage.ShareResponderHandle
import uniffi.ohd_storage.WhoAmIDto
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicReference

/**
 * [StorageBackend] implementation that talks to a remote `ohd-storage-server`
 * over ConnectRPC via the Phase-1 [RemoteOhdStorage] uniffi object.
 *
 * This is the backend selected for every non-`OnDevice` [StorageOption]
 * (`OhdCloud`, `SelfHosted`, `ProviderHosted`). `StorageRepository.openBackend`
 * constructs one of these once the user has signed in through the storage
 * picker (Phase 2 OIDC flow) — there is no local `.ohd` file and no SQLCipher
 * key behind it.
 *
 * # Threading
 *
 * Every [RemoteOhdStorage] method **blocks** the calling thread while the RPC
 * runs (the Rust side hosts its own tokio runtime and `block_on`s). Compose
 * call sites already dispatch storage work off the main thread, but as a
 * belt-and-braces guard [remoteCall] logs loudly if it ever runs on the main
 * looper.
 *
 * # Token refresh
 *
 * The bearer token is a short-lived `ohds_…` self-session access token. When
 * an RPC fails with [OhdException.Auth] whose `code == "TOKEN_EXPIRED"`,
 * [remoteCall] asks [OidcManager.refreshAccessToken] for a fresh token, swaps
 * it into the live [RemoteOhdStorage] via `setBearerToken`, and retries the
 * RPC **once**. Terminal auth failures (`TOKEN_REVOKED`, `WRONG_TOKEN_KIND`,
 * `OUT_OF_SCOPE`) are surfaced as a [Result.failure] carrying a
 * [RemoteAuthException] so the UI can route the user back to sign-in. The
 * retry never loops.
 *
 * # Error mapping
 *
 * The local backend never failed for network reasons; this one can. Transport
 * failures map to [OhdException.Internal] with `code == "UNAVAILABLE"` on the
 * Rust side — [remoteCall] folds every throwable into [Result.failure] with a
 * human-readable message so the ~39 `StorageRepository` `Result` call sites
 * keep working unchanged (no caller crashes on a dropped connection).
 *
 * # Relay-share responder ops
 *
 * `registerRemoteShare` / `startShareResponder` host a *local* relay tunnel
 * responder (the CORD data-link). A remote storage server has no on-device
 * relay responder to host, so those ops return a clear [Result.failure]
 * rather than pretending to succeed — see [unsupportedRemote].
 */
internal class RemoteStorageBackend(
    private val appContext: Context,
    private val remote: RemoteOhdStorage,
) : StorageBackend {

    /**
     * Cached `whoami()` snapshot. The remote backend has no local `user_ulid`
     * to read, so [userUlidOrNull] resolves it via the `WhoAmI` RPC and caches
     * the result for the life of the backend (the bound identity doesn't
     * change across a token refresh — only the token's freshness does).
     */
    @Volatile
    private var whoamiCache: WhoAmIDto? = null

    // -------------------------------------------------------------------------
    // Remote-call shell — off-main-thread guard, error mapping, token refresh.
    // -------------------------------------------------------------------------

    /**
     * Run a blocking [RemoteOhdStorage] RPC, folding the outcome into a
     * [Result]. On a `TOKEN_EXPIRED` auth error, refresh the access token,
     * swap it in, and retry exactly once.
     */
    private inline fun <T> remoteCall(block: RemoteOhdStorage.() -> T): Result<T> {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            Log.w(
                TAG,
                "remote RPC on the main thread — this blocks the UI. " +
                    "Call sites should dispatch to Dispatchers.IO.",
            )
        }
        return runCatching { remote.block() }.recoverCatching { err ->
            if (err is OhdException.Auth && err.code == CODE_TOKEN_EXPIRED) {
                if (!refreshBearerToken()) {
                    throw RemoteAuthException(
                        code = err.code,
                        message = "Session expired and could not be refreshed — sign in again.",
                    )
                }
                // One retry with the refreshed bearer.
                remote.block()
            } else {
                throw mapError(err)
            }
        }.recoverCatching { err ->
            // The retry above may itself fail — map that too.
            throw mapError(err)
        }
    }

    /**
     * Synchronously refresh the persisted self-session access token via
     * AppAuth, then swap the fresh bearer into the live [RemoteOhdStorage].
     *
     * [OidcManager.refreshAccessToken] is callback-based and fires on the main
     * looper, so we bridge it to a blocking call with a latch — safe because
     * [remoteCall] already runs off the main thread.
     *
     * @return `true` iff a fresh token was obtained and applied.
     */
    private fun refreshBearerToken(): Boolean {
        val latch = CountDownLatch(1)
        val ok = AtomicReference(false)
        OidcManager.refreshAccessToken(appContext) { result ->
            ok.set(result.isSuccess)
            latch.countDown()
        }
        runCatching { latch.await() }
        if (!ok.get()) {
            Log.w(TAG, "token refresh failed — remote session needs re-login")
            return false
        }
        val fresh = Auth.getSelfSessionToken(appContext)
        if (fresh.isNullOrEmpty()) {
            Log.w(TAG, "token refresh reported success but no token persisted")
            return false
        }
        remote.setBearerToken(fresh)
        return true
    }

    /**
     * Translate an FFI / transport throwable into one carrying a message the
     * UI can show. Terminal auth failures become a [RemoteAuthException] so
     * the caller can distinguish "re-login needed" from a transient network
     * blip.
     */
    private fun mapError(err: Throwable): Throwable = when (err) {
        is RemoteAuthException -> err
        is OhdException.Auth -> when (err.code) {
            CODE_TOKEN_EXPIRED,
            "TOKEN_REVOKED",
            "WRONG_TOKEN_KIND",
            "OUT_OF_SCOPE",
            -> RemoteAuthException(
                code = err.code,
                message = "Remote storage rejected the session (${err.code}). Sign in again.",
            )
            else -> RemoteAuthException(
                code = err.code,
                message = err.message ?: "Remote storage authentication failed.",
            )
        }
        is OhdException.Internal -> if (err.code == "UNAVAILABLE") {
            // Include the underlying transport detail (TLS/DNS/connect) — the
            // generic text alone hides the real cause when debugging on device.
            Log.w(TAG, "remote transport UNAVAILABLE: ${err.message}")
            RemoteStorageException(
                "Remote storage is unreachable — ${err.message ?: "check your connection"}",
            )
        } else {
            RemoteStorageException(err.message ?: "Remote storage error (${err.code}).")
        }
        is OhdException -> RemoteStorageException(
            err.message ?: "Remote storage operation failed.",
        )
        else -> RemoteStorageException(
            err.message ?: "Remote storage operation failed (${err::class.simpleName}).",
        )
    }

    /**
     * Uniform failure for operations the remote backend cannot perform — the
     * relay-share responder ops, which only make sense for on-device storage
     * hosting its own relay tunnel.
     */
    private fun <T> unsupportedRemote(op: String): Result<T> = Result.failure(
        RemoteStorageException(
            "$op is not available on remote storage — the relay share responder " +
                "runs only on on-device storage.",
        ),
    )

    // --- Events --------------------------------------------------------------

    override fun putEvent(input: EventInput): Result<PutEventOutcome> = remoteCall {
        putEvent(input.toDto()).toDomain()
    }

    override fun putEvents(
        inputs: List<EventInput>,
        atomic: Boolean,
    ): Result<List<PutEventOutcome>> = remoteCall {
        putEvents(inputs.map { it.toDto() }, atomic).map { it.toDomain() }
    }

    override fun queryEvents(filter: EventFilter): Result<List<OhdEvent>> = remoteCall {
        queryEvents(filter.toDto()).map { it.toDomain() }
    }

    override fun listEventTypes(filter: EventFilter): Result<List<EventTypeSummary>> = remoteCall {
        listEventTypes(filter.toDto()).map { EventTypeSummary(it.eventType, it.count) }
    }

    override fun countSources(filter: EventFilter): Result<Long> = remoteCall {
        countSources(filter.toDto()).toLong()
    }

    override fun countEvents(filter: EventFilter): Result<Long> = remoteCall {
        countEvents(filter.toDto()).toLong()
    }

    override fun softDeleteEventsBefore(cutoffMs: Long): Result<Long> =
        // The OHDC service has no soft-delete RPC; free-tier retention is an
        // on-device-only concern (server-hosted plans manage retention
        // server-side). The free-tier worker is gated to `OnDevice` mode, so
        // this is defensive — surface a clear failure rather than crash.
        unsupportedRemote("Retention sweep")

    override fun hardDeleteEventsInRange(
        fromMs: Long,
        toMs: Long,
        eventTypes: List<String>,
    ): Result<Long> = remoteCall {
        // The OHDC `DeleteEvents` RPC accepts nullable bounds; the
        // backend's `fromMs/toMs: Long?` lines up with the uniffi-local
        // method's `Long` (always supplied for the per-event delete
        // path) by promoting them to Some.
        deleteEvents(fromMs, toMs, eventTypes).toLong()
    }

    /**
     * Hard-delete events on the remote server (`DeleteEvents` RPC). Empty
     * filter wipes every event the signed-in identity owns. Returns the
     * count of `events` rows removed.
     */
    fun deleteRemoteEvents(
        fromMs: Long?,
        toMs: Long?,
        eventTypes: List<String>,
    ): Result<Long> = remoteCall {
        deleteEvents(fromMs, toMs, eventTypes).toLong()
    }

    // --- Agent tools ---------------------------------------------------------
    //
    // `listTools` / `executeTool` now ride the OHDC `ListTools` / `ExecuteTool`
    // RPCs the server adds on top of `ohd-mcp-core`. Same JSON shape the
    // local backend returns, so CORD-on-OHD-Cloud uses the catalog
    // identically to CORD-on-device.

    override fun listToolsJson(): Result<String> = remoteCall { listTools() }

    override fun executeToolJson(name: String, inputJson: String): Result<String> =
        remoteCall { executeTool(name, inputJson) }

    // --- Remote access (CORD data-link share responder) ----------------------

    override fun registerRemoteShare(
        grantUlid: String,
        relayOrigin: String,
        identityKeyHex: String,
        shareLabel: String?,
    ): Result<RemoteShareDto> = unsupportedRemote("Activating remote access")

    override fun startShareResponder(
        grantUlid: String,
        share: RemoteShareDto,
        relayTunnelUrl: String,
        identityKeyHex: String,
        allowInsecureDev: Boolean,
    ): Result<ShareResponderHandle> = unsupportedRemote("The share responder")

    // --- Grants --------------------------------------------------------------

    override fun listGrants(includeRevoked: Boolean): Result<List<GrantSummary>> = remoteCall {
        val filter = ListGrantsFilterDto(
            includeRevoked = includeRevoked,
            includeExpired = false,
            granteeKind = null,
            limit = null,
        )
        listGrants(filter).map { it.toDomain() }
    }

    override fun createGrant(input: CreateGrantInput): Result<CreateGrantResult> = remoteCall {
        createGrant(input.toDto()).toDomain()
    }

    override fun revokeGrant(grantUlid: String, reason: String?): Result<Long> =
        // The Phase-1 `RemoteOhdStorage` surface does not yet expose
        // `RevokeGrant`; grant management against remote storage lands with a
        // later phase of the OHDC client crate.
        unsupportedRemote("Revoking a grant")

    override fun updateGrant(
        grantUlid: String,
        label: String?,
        expiresAtMs: Long?,
    ): Result<Unit> = unsupportedRemote("Updating a grant")

    override fun setGrantSuspended(grantUlid: String, suspended: Boolean): Result<Unit> =
        unsupportedRemote("Suspending a grant")

    override fun getGrant(grantUlid: String): Result<GrantSummary?> = remoteCall {
        // No single-grant getter on the wire — list including revoked/expired
        // and pick the row, mirroring the local backend.
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
    ): Result<List<PendingSummary>> = remoteCall {
        listPending().map { it.toDomain() }
    }

    override fun approvePending(pendingUlid: String, alsoTrustType: Boolean): Result<String> =
        // `ApprovePending` / `RejectPending` are not on the Phase-1 remote
        // surface yet.
        unsupportedRemote("Approving a pending event")

    override fun rejectPending(pendingUlid: String, reason: String?): Result<Long> =
        unsupportedRemote("Rejecting a pending event")

    // --- Cases ---------------------------------------------------------------

    override fun listCases(includeClosed: Boolean): Result<List<CaseSummary>> = remoteCall {
        val filter: CaseStateDto? = if (includeClosed) null else CaseStateDto.OPEN
        listCases(filter).map { it.toDomain() }
    }

    override fun getCase(caseUlid: String): Result<CaseDetail> = remoteCall {
        // The remote `get_case` returns a bare `CaseDto`; the local backend's
        // richer `CaseDetailDto` (audit + handoff chain) has no remote
        // equivalent yet. Surface the case header with empty audit/timeline.
        val case = getCase(caseUlid).toDomain()
        CaseDetail(
            ulid = case.ulid,
            caseType = case.caseType,
            label = case.label,
            startedAtMs = case.startedAtMs,
            endedAtMs = case.endedAtMs,
            authorityLabel = case.authorityLabel,
            autoGranted = case.autoGranted,
            timeline = emptyList(),
            audit = emptyList(),
            handoffChain = emptyList(),
        )
    }

    override fun forceCloseCase(caseUlid: String, reason: String?): Result<Long> =
        unsupportedRemote("Force-closing a case")

    override fun issueRetrospectiveGrant(
        caseUlid: String,
        input: CreateGrantInput,
    ): Result<CreateGrantResult> = unsupportedRemote("Issuing a retrospective grant")

    // --- Audit ---------------------------------------------------------------

    override fun auditQuery(filter: AuditFilter): Result<List<AuditEntry>> = remoteCall {
        val kindsForServer = if (filter.opKindsIn.size == 1) filter.opKindsIn.first() else null
        val dto = AuditFilterDto(
            fromMs = filter.fromMs,
            toMs = filter.toMs,
            actorType = null,
            action = kindsForServer,
            result = null,
            limit = filter.limit,
        )
        auditQuery(dto)
            .map { it.toDomain() }
            .filter { e -> filter.opKindsIn.isEmpty() || filter.opKindsIn.contains(e.opKind) }
            .filter { e -> filter.grantUlid == null || e.actorType == "grant" }
    }

    // --- Self-session token --------------------------------------------------

    override fun issueSelfSessionToken(): Result<String> =
        // The self-session token IS the remote bearer — it is minted by the
        // OIDC sign-in flow (Phase 2), not by the storage server. Re-issuing
        // is the OIDC refresh path, not a storage RPC.
        unsupportedRemote("Issuing a self-session token")

    // --- Export --------------------------------------------------------------

    override fun exportAll(): Result<ByteArray> =
        // Phase-1 `RemoteOhdStorage.export()` returns only a frame count as a
        // reachability proof; assembling a portable `.ohd` buffer from the
        // `ExportChunk` framing is a later-phase deliverable.
        unsupportedRemote("Portable .ohd export")

    override fun generateDoctorPdf(): Result<String> =
        unsupportedRemote("Doctor PDF")

    // --- Emergency config ----------------------------------------------------

    override fun getEmergencyConfig(): EmergencyConfigSnapshot? =
        // The OHDC client crate does not yet expose Get/SetEmergencyConfig.
        // `StorageRepository.getEmergencyConfig` falls back to the local
        // `EncryptedSharedPreferences` cache when this returns null.
        null

    override fun setEmergencyConfig(cfg: EmergencyConfig) {
        // No-op: see getEmergencyConfig. The local cache write in
        // `StorageRepository.setEmergencyConfig` still happens.
    }

    // --- Identity facts ------------------------------------------------------

    override fun userUlidOrNull(): String? = runCatching {
        whoami()?.userUlid
    }.getOrNull()

    override fun formatVersionOrNull(): String? =
        // No on-device storage file — there is no local format version.
        "(remote storage)"

    override fun protocolVersionOrNull(): String? = runCatching {
        remoteCall { protocolVersion() }.getOrNull()
    }.getOrNull()

    /**
     * Resolve (and cache) the bearer token's identity via the `WhoAmI` RPC.
     * Returns `null` when the server is unreachable.
     */
    fun whoami(): WhoAmIDto? {
        whoamiCache?.let { return it }
        return remoteCall { whoami() }.getOrNull()?.also { whoamiCache = it }
    }

    companion object {
        private const val TAG = "OhdConnect.RemoteStorage"

        /** OHDC error code signalling a refreshable (non-terminal) stale bearer. */
        private const val CODE_TOKEN_EXPIRED = "TOKEN_EXPIRED"
    }
}

/**
 * Transport / RPC failure from a [RemoteStorageBackend] op, carrying a
 * UI-presentable message. `StorageRepository` call sites already wrap results
 * in `runCatching` / `Result`, so this surfaces as a `Result.failure` exactly
 * like a local backend error would.
 */
class RemoteStorageException(message: String) : Exception(message)

/**
 * Terminal authentication failure from remote storage — the persisted session
 * token is expired-and-unrefreshable, revoked, the wrong kind, or out of
 * scope. The UI should route the user back to the storage sign-in flow.
 */
class RemoteAuthException(
    val code: String,
    message: String,
) : Exception(message)
