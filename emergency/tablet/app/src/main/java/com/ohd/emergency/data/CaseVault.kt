package com.ohd.emergency.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.util.UUID

/**
 * In-memory active-case state machine.
 *
 * Per `SPEC.md` "Trust boundary":
 *
 *     Active case grant tokens — issued by the patient phone after
 *     break-glass, scoped to one case, expire on case close.
 *     Memory-only; not persisted to disk on the tablet.
 *
 *     Patient OHDC data — Cached in tablet RAM during active case;
 *     flushed on case close, app background past N minutes, panic
 *     logout.
 *
 * The vault is the single source of truth for:
 *   - The currently-active case (one at a time per tablet — paramedic
 *     crews work one patient at a time).
 *   - The cached event slice for that case (allergies, meds, vitals,
 *     etc.) — populated on grant from `OhdcService.QueryEvents`.
 *   - The queue of intervention writes pending OHDC submission. When
 *     the relay is unreachable (dead zone, slow handover), interventions
 *     queue here with their `timestamp_ms` stamped at recording time;
 *     replay on reconnect is idempotent because the storage core's
 *     [PutEventOutcome] keys de-duplicate by `(timestamp, ulid)`.
 *
 * Future iteration:
 *  - Back the queued-writes list with a SQLite (uniffi-cached) table so
 *    the queue survives the tablet being rebooted mid-shift. The
 *    patient-data slice stays RAM-only; only the queue persists, and
 *    only its un-submitted rows. See `data_extraction_rules.xml` for
 *    the matching backup-exclusion rule.
 *  - Surface a [Flow]<Status> for the SyncIndicator component.
 */
object CaseVault {

    /** The break-glass state machine, surfaced to the UI as a [StateFlow]. */
    sealed interface BreakGlassState {
        /** Initial state. No request in flight. */
        data object Idle : BreakGlassState

        /** Request sent; waiting for the patient to respond / the timeout to fire. */
        data class Waiting(
            val patientBeaconId: String,
            val operatorLabel: String,
            val responderLabel: String,
            val sentAtMs: Long,
            /** Patient's configured timeout (default 30s; tablet learns this on send). */
            val timeoutSeconds: Int,
            /** Whether the patient's setting is "allow on timeout" (true) or "refuse on timeout" (false). */
            val patientAllowOnTimeout: Boolean,
        ) : BreakGlassState

        /** Granted by the patient (or auto-granted via timeout). */
        data class Granted(
            val patientBeaconId: String,
            val caseUlid: String,
            val grantToken: String,
            val grantedAtMs: Long,
            val autoGranted: Boolean,
        ) : BreakGlassState

        /** Patient explicitly rejected. */
        data class Rejected(
            val patientBeaconId: String,
            val rejectedAtMs: Long,
        ) : BreakGlassState

        /** Patient set "refuse on timeout" and didn't react in time. */
        data class TimedOut(
            val patientBeaconId: String,
            val timedOutAtMs: Long,
        ) : BreakGlassState
    }

    /** A single in-flight or active case. */
    data class ActiveCase(
        val caseUlid: String,
        val patientBeaconId: String,
        val patientLabel: String,
        val openedAtMs: Long,
        val grantToken: String,
        val autoGranted: Boolean,
        /** True once the case has been handed off; further writes refused. */
        val handedOff: Boolean = false,
        val receivingFacility: String? = null,
    )

    /** A queued intervention write that hasn't been flushed to OHDC yet. */
    data class QueuedWrite(
        val localUlid: String,
        val caseUlid: String,
        val occurredAtMs: Long,
        val recordedAtMs: Long,
        val kind: InterventionKind,
        val summary: String,
        /** The actual EventInput payload — dispatched verbatim on flush. */
        val payload: InterventionPayload,
    )

    enum class InterventionKind { Vital, Drug, Observation, Note }

    /** Discriminated union of intervention payloads. */
    sealed interface InterventionPayload {
        data class Vital(
            val channel: String,                  // "vital.hr", "vital.spo2", "vital.bp_sys", ...
            val value: Double,
            val unit: String,
        ) : InterventionPayload

        data class BloodPressure(
            val systolic: Int,
            val diastolic: Int,
        ) : InterventionPayload

        data class Drug(
            val name: String,
            val doseValue: Double,
            val doseUnit: String,
            val route: String,
        ) : InterventionPayload

        data class Observation(
            val freeText: String,
            val gcs: Int? = null,
            val skinColor: String? = null,
        ) : InterventionPayload

        data class Note(val text: String) : InterventionPayload
    }

    /** Sync state surfaced by the top-bar [SyncIndicator]. */
    enum class SyncStatus { Synced, Queued, Syncing, OfflineNoQueue }

    private val _breakGlass = MutableStateFlow<BreakGlassState>(BreakGlassState.Idle)
    val breakGlass: StateFlow<BreakGlassState> = _breakGlass.asStateFlow()

    private val _activeCase = MutableStateFlow<ActiveCase?>(null)
    val activeCase: StateFlow<ActiveCase?> = _activeCase.asStateFlow()

    private val _queuedWrites = MutableStateFlow<List<QueuedWrite>>(emptyList())
    val queuedWrites: StateFlow<List<QueuedWrite>> = _queuedWrites.asStateFlow()

    private val _syncStatus = MutableStateFlow(SyncStatus.Synced)
    val syncStatus: StateFlow<SyncStatus> = _syncStatus.asStateFlow()

    /**
     * Optional persistent backing store for [_queuedWrites]. Set by
     * [EmergencyRepository.init] once the app context is available.
     * Tests + JVM unit-test runs leave it null and the queue stays
     * memory-only.
     */
    @Volatile
    private var persistentStore: QueuedWriteStore? = null

    /** Wire the persistent store. Idempotent; loads any existing rows on first call. */
    @JvmStatic
    fun attachPersistentStore(store: QueuedWriteStore) {
        if (persistentStore != null) return
        persistentStore = store
        val rows = runCatching { store.loadAll() }.getOrDefault(emptyList())
        if (rows.isNotEmpty()) {
            _queuedWrites.value = rows
            _syncStatus.value = SyncStatus.Queued
        }
    }

    // -----------------------------------------------------------------
    // Break-glass state-machine transitions.
    // -----------------------------------------------------------------

    /** Transition Idle → Waiting after the relay accepts our `/emergency/initiate`. */
    fun startWaiting(
        patientBeaconId: String,
        operatorLabel: String,
        responderLabel: String,
        timeoutSeconds: Int = 30,
        patientAllowOnTimeout: Boolean = true,
    ) {
        _breakGlass.value = BreakGlassState.Waiting(
            patientBeaconId = patientBeaconId,
            operatorLabel = operatorLabel,
            responderLabel = responderLabel,
            sentAtMs = System.currentTimeMillis(),
            timeoutSeconds = timeoutSeconds,
            patientAllowOnTimeout = patientAllowOnTimeout,
        )
    }

    /** Patient approved (or timeout-default-allow fired). Opens the active case. */
    fun grantApproved(
        patientBeaconId: String,
        patientLabel: String,
        caseUlid: String = generateLocalCaseUlid(),
        grantToken: String = generateLocalGrantToken(),
        autoGranted: Boolean = false,
    ) {
        val now = System.currentTimeMillis()
        _breakGlass.value = BreakGlassState.Granted(
            patientBeaconId = patientBeaconId,
            caseUlid = caseUlid,
            grantToken = grantToken,
            grantedAtMs = now,
            autoGranted = autoGranted,
        )
        _activeCase.value = ActiveCase(
            caseUlid = caseUlid,
            patientBeaconId = patientBeaconId,
            patientLabel = patientLabel,
            openedAtMs = now,
            grantToken = grantToken,
            autoGranted = autoGranted,
        )
    }

    fun grantRejected(patientBeaconId: String) {
        _breakGlass.value = BreakGlassState.Rejected(
            patientBeaconId = patientBeaconId,
            rejectedAtMs = System.currentTimeMillis(),
        )
    }

    fun grantTimedOut(patientBeaconId: String) {
        _breakGlass.value = BreakGlassState.TimedOut(
            patientBeaconId = patientBeaconId,
            timedOutAtMs = System.currentTimeMillis(),
        )
    }

    fun resetBreakGlass() {
        _breakGlass.value = BreakGlassState.Idle
    }

    // -----------------------------------------------------------------
    // Intervention queue.
    // -----------------------------------------------------------------

    /**
     * Append an intervention to the queue. Returns the locally-assigned
     * ULID so the UI can surface it on the timeline immediately.
     *
     * v0: the queue stays in memory; nothing is actually flushed to
     * OHDC. The caller [EmergencyRepository.submitIntervention] is the
     * place where the real flush will hook in.
     */
    fun enqueueIntervention(
        kind: InterventionKind,
        summary: String,
        payload: InterventionPayload,
        occurredAtMs: Long = System.currentTimeMillis(),
    ): QueuedWrite {
        val active = requireNotNull(_activeCase.value) {
            "Cannot enqueue intervention without an active case"
        }
        val write = QueuedWrite(
            localUlid = generateLocalEventUlid(),
            caseUlid = active.caseUlid,
            occurredAtMs = occurredAtMs,
            recordedAtMs = System.currentTimeMillis(),
            kind = kind,
            summary = summary,
            payload = payload,
        )
        _queuedWrites.update { it + write }
        runCatching { persistentStore?.insert(write) }
        // Optimistic: assume online for now. Real impl flips to Syncing
        // when a flush starts and back to Synced/Queued on completion.
        _syncStatus.value = SyncStatus.Queued
        return write
    }

    /**
     * Mark a single queued write as flushed (drop from queue + persistent
     * store). Used by [EmergencyRepository.submitIntervention] when the
     * OHDC `PutEvents` round-trip succeeds.
     */
    fun markFlushed(localUlid: String) {
        _queuedWrites.update { list -> list.filterNot { it.localUlid == localUlid } }
        runCatching { persistentStore?.deleteByLocalUlid(localUlid) }
        if (_queuedWrites.value.isEmpty()) {
            _syncStatus.value = SyncStatus.Synced
        }
    }

    /** Mark every queued write as flushed; clear the queue. */
    fun markAllFlushed() {
        _queuedWrites.value = emptyList()
        runCatching { persistentStore?.deleteAll() }
        _syncStatus.value = SyncStatus.Synced
    }

    fun setSyncStatus(status: SyncStatus) {
        _syncStatus.value = status
    }

    // -----------------------------------------------------------------
    // Handoff / close.
    // -----------------------------------------------------------------

    fun markHandedOff(receivingFacility: String) {
        _activeCase.update { current ->
            current?.copy(handedOff = true, receivingFacility = receivingFacility)
        }
    }

    /** Drop everything. Called on panic-logout, sign-out, and after handoff. */
    fun clear() {
        _breakGlass.value = BreakGlassState.Idle
        _activeCase.value = null
        _queuedWrites.value = emptyList()
        runCatching { persistentStore?.deleteAll() }
        _syncStatus.value = SyncStatus.Synced
    }

    // -----------------------------------------------------------------
    // Private helpers — local ULIDs for the offline / mock flow.
    // -----------------------------------------------------------------

    /**
     * Local ULID generation.
     *
     * Real ULIDs are 26-char Crockford-base32 of a 48-bit timestamp +
     * 80 random bits (RFC TBD). For the v0 mock we use a UUID prefix
     * that's clearly fake — easy to grep in logs and obviously not a
     * real OHDC ULID. The OHDC client replaces these on flush.
     */
    private fun generateLocalCaseUlid(): String =
        "01CASE${UUID.randomUUID().toString().replace("-", "").uppercase().take(20)}"

    private fun generateLocalEventUlid(): String =
        "01EVT${UUID.randomUUID().toString().replace("-", "").uppercase().take(21)}"

    private fun generateLocalGrantToken(): String =
        "ohdg_DEV_STUB_GRANT_${System.currentTimeMillis()}"
}
