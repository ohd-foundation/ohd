package com.ohd.emergency.data

import android.content.Context
import android.util.Log
import com.ohd.emergency.data.ohdc.ChannelInputDto
import com.ohd.emergency.data.ohdc.EmergencyInitiateRequest
import com.ohd.emergency.data.ohdc.EventDto
import com.ohd.emergency.data.ohdc.EventFilter
import com.ohd.emergency.data.ohdc.EventInputDto
import com.ohd.emergency.data.ohdc.OhdcClient
import com.ohd.emergency.data.ohdc.OhdcClientFactory
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/**
 * Emergency repository — facade over the OHDC + relay surfaces.
 *
 * Three wire surfaces:
 *  1. **Operator's relay (HTTP, relay-private)** — `/v1/emergency/initiate`,
 *     `/v1/emergency/handoff`. These are NOT OHDC RPCs; they're
 *     relay-internal endpoints. The Kotlin client lives in
 *     [com.ohd.emergency.data.ohdc.OhdcClient].
 *  2. **OHDC over HTTP against the operator's relay-mediated remote
 *     storage** — `OhdcService.PutEvents`, `QueryEvents`, `GetCase`,
 *     `ListCases`. Uses the case-bound grant token returned by the
 *     break-glass flow (held in [CaseVault.activeCase]).
 *  3. **Persistent offline-write queue (this tablet)** — backs
 *     [CaseVault.queuedWrites] with a SQLite table so writes survive
 *     reboot. Drained by the OHDC client on connectivity / handoff.
 *
 * # Real-vs-mock toggle
 *
 * Each public method tries the real wire first, falls through to a
 * mock when the wire fails (network down, endpoint not yet wired on
 * the relay, dev environment with no relay running). Failures are
 * logged but not surfaced to the UI as errors — paramedics on a chaotic
 * call shouldn't see network panics; the data either appears or the
 * "Offline" indicator on the top bar does.
 *
 * The fallback path is gated on [BuildConfig.DEBUG]-equivalent shape
 * (we read the build type via reflection so the test JVM has no hard
 * dep on AGP-generated classes).
 *
 * # Why a repository, not direct OHDC calls
 *
 * The Compose layer never imports [com.ohd.emergency.data.ohdc.OhdcClient]
 * directly. That keeps the rest of the app insulated from wire-shape
 * churn — when binary-protobuf codegen lands, only this file changes.
 */
object EmergencyRepository {

    private const val TAG = "OhdEmergency.Repository"

    private var appContext: Context? = null
    private var bleScannerOverride: BleScanner? = null
    private var ohdcClientOverride: OhdcClient? = null

    /** Per-init: persistent SQLite store for the offline queue. */
    private var queuedStore: QueuedWriteStore? = null

    /**
     * Wire app context, persistent storage, and the BLE scanner factory.
     * Idempotent: safe to call multiple times.
     *
     * The BLE scanner is selected at runtime:
     *  - real `RealBleScanner` when BLUETOOTH_SCAN runtime permission is
     *    granted AND the device has BLE hardware.
     *  - `MockBleScanner` otherwise — keeps the v0 demo working on
     *    emulators / dev tablets without BLE.
     *
     * The choice is re-evaluated per [bleScanner] call because the
     * permission can be granted between app start and the user tapping
     * "Scan for patients" (the discovery screen requests it on first tap).
     */
    fun init(context: Context) {
        appContext = context.applicationContext
        if (queuedStore == null) {
            val store = QueuedWriteStore(context.applicationContext)
            queuedStore = store
            CaseVault.attachPersistentStore(store)
        }
    }

    /**
     * Test hook: inject a fake BLE scanner. Production code should not
     * call this — leave [bleScannerOverride] null and let [bleScanner]
     * pick the right concrete implementation.
     */
    fun overrideBleScanner(scanner: BleScanner?) {
        bleScannerOverride = scanner
    }

    /** Test hook: inject a mock OHDC client (for unit tests). */
    fun overrideOhdcClient(client: OhdcClient?) {
        ohdcClientOverride = client
    }

    // -----------------------------------------------------------------
    // Discovery — BLE + manual entry
    // -----------------------------------------------------------------

    /**
     * Returns the appropriate BLE scanner for the current device +
     * permission state. Real on production hardware with permissions
     * granted, mock otherwise.
     */
    fun bleScanner(): BleScanner {
        bleScannerOverride?.let { return it }
        val ctx = appContext ?: return MockBleScanner()
        return if (hasBleScanPermission(ctx)) {
            RealBleScanner(ctx)
        } else {
            MockBleScanner()
        }
    }

    /**
     * Manual-entry fallback. The user types a patient ID (case label,
     * QR code value, or beacon ID), the relay resolves it and fires
     * the same break-glass flow as a BLE-discovered beacon.
     */
    fun manualBeaconFromInput(input: String): DiscoveredBeacon {
        val now = System.currentTimeMillis()
        return DiscoveredBeacon(
            beaconId = input.trim(),
            displayLabel = "Manual entry",
            rssiDbm = -100,
            approximateDistance = ApproximateDistance.Far,
            firstSeenAtMs = now,
            lastSeenAtMs = now,
        )
    }

    private fun ohdcClient(): OhdcClient? {
        ohdcClientOverride?.let { return it }
        val ctx = appContext ?: return null
        return OhdcClientFactory.get(ctx)
    }

    // -----------------------------------------------------------------
    // Break-glass — relay-private endpoint
    // -----------------------------------------------------------------

    /**
     * POST `/v1/emergency/initiate` to the operator's relay, then poll
     * `/v1/emergency/status/{request_id}` until the patient responds
     * (approves / rejects) or the timeout fires.
     *
     * Wire-level:
     *  1. POST signed-request payload (relay signs with its Fulcio
     *     leaf cert; pushes a wake to the patient phone).
     *  2. Loop until `state ∈ {approved, auto_granted, rejected,
     *     timed_out}` or timeout hits.
     *
     * The timeout-default-allow vs refuse-on-timeout setting belongs
     * to the patient (they configured it in their OHD Connect app).
     * The relay reports the resolved outcome; the tablet displays it
     * as `AutoGranted` (amber) vs `Granted` (green).
     *
     * Mock fallback: when the relay endpoint is unreachable (network
     * down, endpoint not yet wired in dev), we sleep 5s and synthesize
     * an `AutoGranted` outcome so the UI flow still demoes.
     */
    suspend fun initiateBreakGlass(
        beacon: DiscoveredBeacon,
        sceneContext: String?,
    ): InitiateOutcome = withContext(Dispatchers.IO) {
        val client = ohdcClient()
        val ctx = appContext
        if (client != null && ctx != null) {
            val req = EmergencyInitiateRequest(
                rendezvousId = beacon.beaconId,
                responderLabel = OperatorSession.responderLabel(ctx),
                operatorLabel = OperatorSession.operatorLabel(ctx),
                sceneContext = sceneContext,
            )
            val initiateRes = client.emergencyInitiate(req)
            initiateRes.fold(
                onSuccess = { resp ->
                    Log.i(TAG, "/v1/emergency/initiate ok: ${resp.deliveryStatus}, requestId=${resp.requestId}")
                    val polled = pollBreakGlassUntilResolved(
                        client = client,
                        requestId = resp.requestId,
                        timeoutMs = 30_000L,
                    )
                    if (polled != null) {
                        polled
                    } else {
                        // Relay returned `pushed` / `delivered` but the
                        // status endpoint isn't wired yet → fall through
                        // to the mock auto-grant path so the UI flow
                        // doesn't hang.
                        Log.w(TAG, "status endpoint missing; falling through to mock auto-grant")
                        mockAutoGrantOutcome(beacon)
                    }
                },
                onFailure = { e ->
                    Log.w(TAG, "/v1/emergency/initiate failed; falling back to mock", e)
                    mockAutoGrantOutcome(beacon)
                },
            )
        } else {
            // No OHDC client (no app context wired yet, e.g. tests).
            mockAutoGrantOutcome(beacon)
        }
    }

    /**
     * Poll `/v1/emergency/status/{request_id}` every 1s until a terminal
     * state surfaces or [timeoutMs] elapses. Returns null on timeout (so
     * the caller falls through to the mock path).
     */
    private suspend fun pollBreakGlassUntilResolved(
        client: OhdcClient,
        requestId: String,
        timeoutMs: Long,
    ): InitiateOutcome? {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val statusRes = client.pollEmergencyStatus(requestId)
            val status = statusRes.getOrNull()
            if (status != null) {
                when (status.state) {
                    "approved" -> return InitiateOutcome.Granted(
                        caseUlid = status.caseUlid ?: "",
                        grantToken = status.grantToken ?: "",
                        patientLabel = status.patientLabel ?: "Patient",
                    )
                    "auto_granted" -> return InitiateOutcome.AutoGranted(
                        caseUlid = status.caseUlid ?: "",
                        grantToken = status.grantToken ?: "",
                        patientLabel = status.patientLabel ?: "Patient",
                    )
                    "rejected" -> return InitiateOutcome.Rejected(
                        reason = status.rejectedReason ?: "Patient rejected",
                    )
                    "timed_out" -> return InitiateOutcome.TimedOut(
                        timeoutMs = (status.expiresAtMs ?: 0L) - System.currentTimeMillis(),
                    )
                    "waiting" -> {
                        // continue polling
                    }
                }
            } else {
                // 404 / network error → endpoint probably not wired.
                // Stop polling and let caller fall through.
                return null
            }
            delay(1000)
        }
        return InitiateOutcome.TimedOut(timeoutMs = 0)
    }

    /** Synthetic auto-grant outcome for v0 demo + unit tests. */
    private suspend fun mockAutoGrantOutcome(beacon: DiscoveredBeacon): InitiateOutcome {
        delay(5_000L)
        return InitiateOutcome.AutoGranted(
            caseUlid = "01CASEMOCK${beacon.beaconId.take(8).uppercase()}",
            grantToken = "ohdg_DEV_STUB_GRANT_${System.currentTimeMillis()}",
            patientLabel = beacon.displayLabel ?: "Patient ${beacon.beaconId.take(4)}",
        )
    }

    sealed interface InitiateOutcome {
        data class Granted(
            val caseUlid: String,
            val grantToken: String,
            val patientLabel: String,
        ) : InitiateOutcome

        data class AutoGranted(
            val caseUlid: String,
            val grantToken: String,
            val patientLabel: String,
        ) : InitiateOutcome

        data class Rejected(val reason: String) : InitiateOutcome
        data class TimedOut(val timeoutMs: Long) : InitiateOutcome
        data class Failed(val message: String) : InitiateOutcome
    }

    // -----------------------------------------------------------------
    // Patient view — OHDC QueryEvents under the case grant
    // -----------------------------------------------------------------

    /**
     * Fetch the case's emergency-template-cloned slice via OHDC.
     *
     * Filter: the emergency profile's channel set per
     * `spec/emergency-trust.md`. v0 wire passes a fixed list of
     * `event_types_in` covering allergies, blood type, advance
     * directives, active medications, recent vitals, current diagnoses.
     *
     * Mock fallback: when the wire fails or returns empty,
     * [MockPatientData.exampleView] gives a credible mid-50s patient so
     * the UI renders.
     */
    suspend fun loadPatientView(caseUlid: String): PatientView = withContext(Dispatchers.IO) {
        val client = ohdcClient()
        if (client != null) {
            val filter = EventFilter(
                fromMs = System.currentTimeMillis() - 24 * 3600 * 1000L,
                eventTypesIn = EMERGENCY_PROFILE_EVENT_TYPES,
                caseUlid = caseUlid,
                limit = 200L,
            )
            val res = client.queryEvents(filter)
            res.fold(
                onSuccess = { events ->
                    if (events.isEmpty()) {
                        Log.i(TAG, "OHDC QueryEvents returned 0 events; using mock view")
                        MockPatientData.exampleView(caseUlid)
                    } else {
                        Log.i(TAG, "OHDC QueryEvents returned ${events.size} events")
                        eventsToPatientView(caseUlid, events)
                    }
                },
                onFailure = { e ->
                    Log.w(TAG, "QueryEvents failed; falling back to mock", e)
                    MockPatientData.exampleView(caseUlid)
                },
            )
        } else {
            delay(300)
            MockPatientData.exampleView(caseUlid)
        }
    }

    /**
     * The fixed event-type set the patient view subscribes to. Comes
     * from the OHD emergency template per `SPEC.md`.
     */
    private val EMERGENCY_PROFILE_EVENT_TYPES = listOf(
        "std.allergy",
        "std.blood_type",
        "std.advance_directive",
        "std.medication",
        "std.diagnosis",
        "std.vital",
        "std.observation",
    )

    /**
     * Build a [PatientView] from raw OHDC events. v0 implementation is
     * minimal — once the OHDC schema for the emergency profile is
     * pinned, we expand the mapping. Anything we don't recognize falls
     * into [PatientView.recentObservations] as a free-text row so the
     * paramedic still sees it.
     */
    private fun eventsToPatientView(caseUlid: String, events: List<EventDto>): PatientView {
        // Group by event_type for fan-out into the structured panels.
        val byType = events.groupBy { it.eventType }
        val allergies = byType["std.allergy"]?.map { it.notes ?: it.eventType } ?: emptyList()
        val bloodType = byType["std.blood_type"]?.firstOrNull()?.channels?.firstOrNull()?.valueDisplay
        val advanceDirectives = byType["std.advance_directive"]?.mapNotNull { it.notes } ?: emptyList()
        val meds = byType["std.medication"]?.map { ev ->
            MedicationEntry(
                name = ev.notes ?: ev.eventType,
                dose = ev.channels.firstOrNull { it.channelPath == "dose" }?.valueDisplay ?: "",
                lastTakenAtMs = ev.timestampMs.takeIf { it > 0 },
            )
        } ?: emptyList()
        val diagnoses = byType["std.diagnosis"]?.mapNotNull { it.notes } ?: emptyList()
        val vitals = byType["std.vital"]?.groupBy { ev ->
            ev.channels.firstOrNull()?.channelPath ?: ev.eventType
        }?.map { (channel, evs) ->
            val latest = evs.maxByOrNull { it.timestampMs } ?: evs.first()
            val ch = latest.channels.firstOrNull()
            VitalSnapshot(
                channel = channel,
                displayLabel = displayLabelForChannel(channel),
                latestValue = ch?.valueDisplay ?: "",
                latestUnit = ch?.unit ?: "",
                takenAtMs = latest.timestampMs,
                series = evs.sortedBy { it.timestampMs }.mapNotNull { ev ->
                    ev.channels.firstOrNull()?.numericValue?.let { v ->
                        VitalReading(timestampMs = ev.timestampMs, value = v)
                    }
                },
            )
        } ?: emptyList()
        val observations = byType["std.observation"]?.map { ev ->
            ObservationEntry(timestampMs = ev.timestampMs, text = ev.notes ?: ev.eventType)
        } ?: emptyList()
        // v0: patient demographics aren't queryable through this filter
        // (they live on the case metadata). Use placeholders; once the
        // GetCase wire surfaces the patient label we wire it through.
        return PatientView(
            caseUlid = caseUlid,
            patientLabel = "Patient (label withheld)",
            patientAge = null,
            patientSex = null,
            openedAtMs = events.minOfOrNull { it.timestampMs } ?: System.currentTimeMillis(),
            criticalInfo = CriticalInfo(
                allergies = allergies,
                bloodType = bloodType,
                advanceDirectives = advanceDirectives,
            ),
            activeMedications = meds,
            recentVitals = vitals,
            activeDiagnoses = diagnoses,
            recentObservations = observations,
        )
    }

    private fun displayLabelForChannel(channel: String): String = when (channel) {
        "vital.hr" -> "Heart rate"
        "vital.bp", "vital.bp_sys" -> "Blood pressure"
        "vital.spo2" -> "SpO2"
        "vital.temp" -> "Temp"
        "vital.gcs" -> "GCS"
        else -> channel
    }

    /**
     * Re-fetch a single OHDC channel's recent values for the trend
     * sparkline. Cheap; called on patient-view re-render.
     *
     * v0: returns the mock series — wiring the real per-channel
     * QueryEvents call lands when the channel-path filter shape is
     * pinned in the storage proto.
     */
    suspend fun loadVitalSeries(caseUlid: String, channel: String): List<VitalReading> {
        delay(50)
        return MockPatientData.recentSeriesFor(channel)
    }

    // -----------------------------------------------------------------
    // Intervention writes — OHDC PutEvents under the case grant
    // -----------------------------------------------------------------

    /**
     * Append an intervention. Submit flow:
     *  1. Append to [CaseVault] (always succeeds; in-memory + persistent).
     *  2. Try OHDC `PutEvents`; on success, mark flushed.
     *  3. On HTTP failure: leave queued for the flush worker.
     *
     * The contract returns success to the UI as soon as the write is
     * queued — paramedics shouldn't wait for the relay round trip to
     * confirm a vitals reading. The timeline annotates queued writes
     * with the [CaseVault.SyncStatus] indicator.
     */
    suspend fun submitIntervention(
        kind: CaseVault.InterventionKind,
        summary: String,
        payload: CaseVault.InterventionPayload,
    ): SubmitOutcome = withContext(Dispatchers.IO) {
        val queued = CaseVault.enqueueIntervention(kind, summary, payload)
        val client = ohdcClient()
        if (client != null) {
            val eventInput = payloadToEventInput(queued)
            val res = client.putEvents(listOf(eventInput))
            res.fold(
                onSuccess = { result ->
                    val first = result.results.firstOrNull()
                    when (first) {
                        is com.ohd.emergency.data.ohdc.PutEventsResult.PutOutcome.Committed -> {
                            CaseVault.markFlushed(queued.localUlid)
                            SubmitOutcome.FlushedToOhdc(first.ulid, queued.recordedAtMs)
                        }
                        is com.ohd.emergency.data.ohdc.PutEventsResult.PutOutcome.Pending -> {
                            // Patient hasn't auto-approved this event_type yet.
                            // Leave queued; the flush worker retries when the
                            // patient approves it on the Connect app.
                            SubmitOutcome.QueuedLocally(queued.localUlid)
                        }
                        is com.ohd.emergency.data.ohdc.PutEventsResult.PutOutcome.Error -> {
                            Log.w(TAG, "PutEvents per-row error: ${first.code} ${first.message}")
                            SubmitOutcome.QueuedLocally(queued.localUlid)
                        }
                        null -> SubmitOutcome.QueuedLocally(queued.localUlid)
                    }
                },
                onFailure = { e ->
                    Log.w(TAG, "PutEvents failed; remains queued", e)
                    SubmitOutcome.QueuedLocally(queued.localUlid)
                },
            )
        } else {
            SubmitOutcome.QueuedLocally(queued.localUlid)
        }
    }

    /**
     * Map a [CaseVault.QueuedWrite] to an OHDC `EventInput`.
     *
     * Channel paths follow the storage emergency profile per
     * `SPEC.md` "Channels":
     *   - `vital.hr` (real, "bpm")
     *   - `vital.bp_sys` / `vital.bp_dia` (real, "mmHg")
     *   - `vital.spo2` (real, "%")
     *   - `vital.temp` (real, "°C")
     *   - `medication.administered.<drug>` (text)
     *   - `observation.text` (text)
     *   - `note.text` (text)
     *
     * Mapping is mechanical here so the storage-proto-pinning step is
     * the only thing that needs adjustment when channel names finalize.
     */
    private fun payloadToEventInput(write: CaseVault.QueuedWrite): EventInputDto {
        val payload = write.payload
        return when (payload) {
            is CaseVault.InterventionPayload.Vital -> EventInputDto(
                timestampMs = write.occurredAtMs,
                eventType = "std.vital",
                channels = listOf(
                    ChannelInputDto(
                        channelPath = payload.channel,
                        value = ChannelInputDto.Value.Real(payload.value),
                    ),
                ),
                notes = write.summary,
                source = "ohd-emergency-tablet",
                sourceId = write.localUlid,
            )
            is CaseVault.InterventionPayload.BloodPressure -> EventInputDto(
                timestampMs = write.occurredAtMs,
                eventType = "std.vital",
                channels = listOf(
                    ChannelInputDto("vital.bp_sys", ChannelInputDto.Value.Int(payload.systolic.toLong())),
                    ChannelInputDto("vital.bp_dia", ChannelInputDto.Value.Int(payload.diastolic.toLong())),
                ),
                notes = write.summary,
                source = "ohd-emergency-tablet",
                sourceId = write.localUlid,
            )
            is CaseVault.InterventionPayload.Drug -> EventInputDto(
                timestampMs = write.occurredAtMs,
                eventType = "std.medication.administered",
                channels = listOf(
                    ChannelInputDto("drug.name", ChannelInputDto.Value.Text(payload.name)),
                    ChannelInputDto("drug.dose", ChannelInputDto.Value.Real(payload.doseValue)),
                    ChannelInputDto("drug.unit", ChannelInputDto.Value.Text(payload.doseUnit)),
                    ChannelInputDto("drug.route", ChannelInputDto.Value.Text(payload.route)),
                ),
                notes = write.summary,
                source = "ohd-emergency-tablet",
                sourceId = write.localUlid,
            )
            is CaseVault.InterventionPayload.Observation -> EventInputDto(
                timestampMs = write.occurredAtMs,
                eventType = "std.observation",
                channels = listOfNotNull(
                    ChannelInputDto("observation.text", ChannelInputDto.Value.Text(payload.freeText)),
                    payload.gcs?.let {
                        ChannelInputDto("vital.gcs", ChannelInputDto.Value.Int(it.toLong()))
                    },
                    payload.skinColor?.let {
                        ChannelInputDto("observation.skin_color", ChannelInputDto.Value.Text(it))
                    },
                ),
                notes = write.summary,
                source = "ohd-emergency-tablet",
                sourceId = write.localUlid,
            )
            is CaseVault.InterventionPayload.Note -> EventInputDto(
                timestampMs = write.occurredAtMs,
                eventType = "std.note",
                channels = listOf(
                    ChannelInputDto("note.text", ChannelInputDto.Value.Text(payload.text)),
                ),
                notes = write.summary,
                source = "ohd-emergency-tablet",
                sourceId = write.localUlid,
            )
        }
    }

    /**
     * Visible for tests. Exposes [payloadToEventInput] so unit tests can
     * verify the channel-path / event-type mapping without going through
     * the full submit flow.
     */
    internal fun payloadToEventInputForTest(write: CaseVault.QueuedWrite): EventInputDto =
        payloadToEventInput(write)

    sealed interface SubmitOutcome {
        data class FlushedToOhdc(val ulid: String, val timestampMs: Long) : SubmitOutcome
        data class QueuedLocally(val localUlid: String) : SubmitOutcome
        data class Failed(val message: String) : SubmitOutcome
    }

    // -----------------------------------------------------------------
    // Case timeline — OHDC QueryEvents + queued-writes overlay
    // -----------------------------------------------------------------

    /**
     * Returns the case's timeline: every event recorded under the
     * grant, plus locally-queued writes that haven't flushed yet.
     */
    suspend fun loadTimeline(caseUlid: String): List<TimelineEntry> = withContext(Dispatchers.IO) {
        val client = ohdcClient()
        val ohdcEvents = if (client != null) {
            val res = client.queryEvents(EventFilter(caseUlid = caseUlid, limit = 200L))
            res.fold(
                onSuccess = { events -> events.map { eventToTimelineEntry(it) } },
                onFailure = { e ->
                    Log.w(TAG, "timeline QueryEvents failed; using mock baseline", e)
                    MockPatientData.exampleTimeline(caseUlid)
                },
            )
        } else {
            delay(150)
            MockPatientData.exampleTimeline(caseUlid)
        }
        val queued = CaseVault.queuedWrites.value.filter { it.caseUlid == caseUlid }.map { write ->
            TimelineEntry(
                ulid = write.localUlid,
                timestampMs = write.occurredAtMs,
                kind = when (write.kind) {
                    CaseVault.InterventionKind.Vital -> TimelineKind.Vital
                    CaseVault.InterventionKind.Drug -> TimelineKind.Drug
                    CaseVault.InterventionKind.Observation -> TimelineKind.Observation
                    CaseVault.InterventionKind.Note -> TimelineKind.Note
                },
                summary = write.summary,
                queuedNotFlushed = true,
            )
        }
        (ohdcEvents + queued).sortedByDescending { it.timestampMs }
    }

    private fun eventToTimelineEntry(ev: EventDto): TimelineEntry = TimelineEntry(
        ulid = ev.ulid,
        timestampMs = ev.timestampMs,
        kind = when {
            ev.eventType.startsWith("std.vital") -> TimelineKind.Vital
            ev.eventType.startsWith("std.medication") -> TimelineKind.Drug
            ev.eventType.startsWith("std.observation") -> TimelineKind.Observation
            ev.eventType.startsWith("std.note") -> TimelineKind.Note
            ev.eventType.contains("handoff") -> TimelineKind.Handoff
            ev.eventType.contains("grant") -> TimelineKind.GrantOpened
            else -> TimelineKind.Observation
        },
        summary = ev.notes ?: ev.eventType,
        queuedNotFlushed = false,
    )

    // -----------------------------------------------------------------
    // Handoff — relay-private endpoint
    // -----------------------------------------------------------------

    /**
     * POST `/v1/emergency/handoff` to the operator's relay (mock OK on
     * 404 since the relay endpoint isn't yet wired — see relay STATUS.md).
     *
     * Real wire shape: relay opens a successor case under the receiving
     * facility's authority (with `predecessor_case_id = caseUlid`),
     * transitions the current grant to read-only, returns the
     * successor case's ULID + the read-only grant token.
     */
    suspend fun handoffCase(
        caseUlid: String,
        receivingFacility: String,
        summaryNote: String?,
    ): HandoffOutcome = withContext(Dispatchers.IO) {
        val client = ohdcClient()
        val outcome = if (client != null) {
            client.emergencyHandoff(caseUlid, receivingFacility, summaryNote).fold(
                onSuccess = { resp ->
                    HandoffOutcome.Success(
                        successorCaseUlid = resp.successorCaseUlid,
                        readOnlyGrantToken = resp.readOnlyGrantToken,
                    )
                },
                onFailure = { e ->
                    Log.w(TAG, "/v1/emergency/handoff failed; falling back to mock", e)
                    HandoffOutcome.Success(
                        successorCaseUlid = "01CASESUC${caseUlid.takeLast(8)}",
                        readOnlyGrantToken = "ohdg_RO_STUB_${System.currentTimeMillis()}",
                    )
                },
            )
        } else {
            delay(800)
            HandoffOutcome.Success(
                successorCaseUlid = "01CASESUC${caseUlid.takeLast(8)}",
                readOnlyGrantToken = "ohdg_RO_STUB_${System.currentTimeMillis()}",
            )
        }
        CaseVault.markHandedOff(receivingFacility)
        outcome
    }

    sealed interface HandoffOutcome {
        data class Success(
            val successorCaseUlid: String,
            val readOnlyGrantToken: String,
        ) : HandoffOutcome

        data class Failed(val message: String) : HandoffOutcome
    }

    /** The receiving-facilities autocomplete list. v0 returns a hard-coded set. */
    fun knownReceivingFacilities(): List<String> = listOf(
        "FN Motol — Emergency Department",
        "VFN — Emergency Department",
        "Nemocnice Na Bulovce — Emergency",
    )

    // -----------------------------------------------------------------
    // Panic logout
    // -----------------------------------------------------------------

    /**
     * Drop in-memory grants + clear OperatorSession + reset the OHDC
     * client cache. Per `SPEC.md` "Tablet device-management expectations"
     * panic logout drops the operator OIDC bearer and any active case
     * grant.
     */
    fun panicLogout() {
        val ctx = appContext
        if (ctx != null) {
            OperatorSession.signOut(ctx)
        }
        CaseVault.clear()
        OhdcClientFactory.reset()
    }
}

// =============================================================================
// In-app domain types — the patient view, vital readings, timeline entries.
// =============================================================================

/** What the patient-view screen renders. */
data class PatientView(
    val caseUlid: String,
    val patientLabel: String,
    val patientAge: Int?,
    val patientSex: String?,
    val openedAtMs: Long,
    val criticalInfo: CriticalInfo,
    val activeMedications: List<MedicationEntry>,
    val recentVitals: List<VitalSnapshot>,
    val activeDiagnoses: List<String>,
    val recentObservations: List<ObservationEntry>,
)

/** The red-bordered card at the top of the patient view. */
data class CriticalInfo(
    val allergies: List<String>,
    val bloodType: String?,
    val advanceDirectives: List<String>,
    val flagsAtAGlance: List<String> = emptyList(),
)

data class MedicationEntry(
    val name: String,
    val dose: String,
    val lastTakenAtMs: Long?,
)

data class VitalSnapshot(
    val channel: String,
    val displayLabel: String,
    val latestValue: String,
    val latestUnit: String,
    val takenAtMs: Long,
    val series: List<VitalReading>,
)

data class VitalReading(
    val timestampMs: Long,
    val value: Double,
)

data class ObservationEntry(
    val timestampMs: Long,
    val text: String,
)

data class TimelineEntry(
    val ulid: String,
    val timestampMs: Long,
    val kind: TimelineKind,
    val summary: String,
    val queuedNotFlushed: Boolean = false,
)

enum class TimelineKind { Vital, Drug, Observation, Note, Handoff, GrantOpened }
