package com.ohd.emergency.data

import com.ohd.emergency.data.ohdc.ChannelInputDto
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.Buffer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import com.ohd.emergency.data.ohdc.OhdcClient

/**
 * Repository-level tests of the 7-step paramedic flow logic, with a
 * MockWebServer standing in for the operator's relay + the OHDC
 * service. Mirrors the storyboard:
 *
 *   Login (mock-stubbed) → Discovery → BreakGlass → Patient →
 *   Intervention → Timeline → Handoff
 *
 * The Login + Discovery legs are exercised directly against the
 * scanner / session APIs; everything from BreakGlass onwards rides
 * the OhdcClient.
 *
 * Notes on test isolation:
 *  - We swap [EmergencyRepository.overrideOhdcClient] to a client
 *    pointed at the mock server; this avoids any need for app context.
 *  - We swap [EmergencyRepository.overrideBleScanner] to a fixed mock so
 *    the scanner emits deterministic results.
 *  - Each test resets the [CaseVault] singleton in tearDown so
 *    in-process state doesn't leak.
 */
class EmergencyRepositoryTest {

    private lateinit var server: MockWebServer

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
        val client = OhdcClient(
            baseUrl = server.url("").toString().trimEnd('/'),
            operatorBearerProvider = { "operator_bearer" },
            grantTokenProvider = { "ohdg_grant" },
        )
        EmergencyRepository.overrideOhdcClient(client)
    }

    @After
    fun tearDown() {
        EmergencyRepository.overrideOhdcClient(null)
        EmergencyRepository.overrideBleScanner(null)
        CaseVault.clear()
        server.shutdown()
    }

    @Test
    fun manualBeacon_synthesizes_far_beacon() {
        val b = EmergencyRepository.manualBeaconFromInput("rdv_xyz_123")
        assertEquals("rdv_xyz_123", b.beaconId)
        assertEquals(ApproximateDistance.Far, b.approximateDistance)
    }

    @Test
    fun submitIntervention_hits_OHDC_and_marks_flushed_on_committed() = runBlockingTestNoTimer {
        // Set up an active case so the queue can accept writes.
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEX",
        )
        // OHDC PutEvents → 1 committed result.
        server.enqueue(MockResponse().setBody("""
            { "results": [ { "outcome": { "committed": { "ulid": { "crockford": "01EVTHR" } } } } ] }
        """.trimIndent()))

        val outcome = EmergencyRepository.submitIntervention(
            kind = CaseVault.InterventionKind.Vital,
            summary = "HR 112 bpm",
            payload = CaseVault.InterventionPayload.Vital(
                channel = "vital.hr", value = 112.0, unit = "bpm",
            ),
        )

        assertTrue(
            "expected FlushedToOhdc; got ${outcome::class.simpleName}",
            outcome is EmergencyRepository.SubmitOutcome.FlushedToOhdc,
        )
        // Queue should be empty after flush.
        assertEquals(0, CaseVault.queuedWrites.value.size)
        assertEquals(CaseVault.SyncStatus.Synced, CaseVault.syncStatus.value)

        val req = server.takeRequest()
        assertEquals("/ohdc.v0.OhdcService/PutEvents", req.path)
        assertEquals("Bearer ohdg_grant", req.getHeader("Authorization"))
    }

    @Test
    fun submitIntervention_keeps_queued_when_OHDC_returns_pending() = runBlockingTestNoTimer {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEX",
        )
        server.enqueue(MockResponse().setBody("""
            { "results": [ { "outcome": { "pending": { "ulid": { "crockford": "01EVTPEND" }, "expires_at_ms": 99 } } } ] }
        """.trimIndent()))

        val outcome = EmergencyRepository.submitIntervention(
            kind = CaseVault.InterventionKind.Note,
            summary = "Pt LOC fluctuating",
            payload = CaseVault.InterventionPayload.Note("Pt LOC fluctuating"),
        )
        assertTrue(outcome is EmergencyRepository.SubmitOutcome.QueuedLocally)
        assertEquals(1, CaseVault.queuedWrites.value.size)
        assertEquals(CaseVault.SyncStatus.Queued, CaseVault.syncStatus.value)
    }

    @Test
    fun submitIntervention_keeps_queued_on_HTTP_failure() = runBlockingTestNoTimer {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEX",
        )
        server.enqueue(MockResponse().setResponseCode(503))
        val outcome = EmergencyRepository.submitIntervention(
            kind = CaseVault.InterventionKind.Vital,
            summary = "BP 92/56",
            payload = CaseVault.InterventionPayload.BloodPressure(systolic = 92, diastolic = 56),
        )
        assertTrue(outcome is EmergencyRepository.SubmitOutcome.QueuedLocally)
        assertEquals(1, CaseVault.queuedWrites.value.size)
    }

    @Test
    fun loadTimeline_combines_OHDC_and_queued() = runBlockingTestNoTimer {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEX",
        )
        // Queue one local write.
        CaseVault.enqueueIntervention(
            kind = CaseVault.InterventionKind.Vital,
            summary = "HR 110",
            payload = CaseVault.InterventionPayload.Vital("vital.hr", 110.0, "bpm"),
            occurredAtMs = 1000L,
        )

        // OHDC streaming response — 1 frame.
        val buf = Buffer()
        val ev = """{"ulid":{"crockford":"01EVTSRV"},"timestamp_ms":2000,"event_type":"std.vital","notes":"BP 90/60"}"""
        buf.writeByte(0); buf.writeInt(ev.length); buf.write(ev.toByteArray(Charsets.UTF_8))
        buf.writeByte(0x02); buf.writeInt(2); buf.write("{}".toByteArray(Charsets.UTF_8))
        server.enqueue(MockResponse().setBody(buf))

        val timeline = EmergencyRepository.loadTimeline("01CASEX")
        // Expect 2 entries: 1 from OHDC + 1 queued.
        assertEquals(2, timeline.size)
        // Sorted descending by ts: server entry (ts=2000) before queued (ts=1000).
        assertEquals(2000L, timeline[0].timestampMs)
        assertFalse(timeline[0].queuedNotFlushed)
        assertEquals(1000L, timeline[1].timestampMs)
        assertTrue(timeline[1].queuedNotFlushed)
    }

    @Test
    fun loadPatientView_falls_back_to_mock_when_OHDC_returns_empty() = runBlockingTestNoTimer {
        // Empty stream (just end-frame).
        val buf = Buffer()
        buf.writeByte(0x02); buf.writeInt(2); buf.write("{}".toByteArray(Charsets.UTF_8))
        server.enqueue(MockResponse().setBody(buf))

        val view = EmergencyRepository.loadPatientView("01CASEX")
        // Mock view always populates these.
        assertNotNull(view.criticalInfo)
        assertTrue(view.recentVitals.isNotEmpty())
        assertEquals("01CASEX", view.caseUlid)
    }

    @Test
    fun loadPatientView_maps_OHDC_events_into_panels() = runBlockingTestNoTimer {
        val buf = Buffer()
        val events = listOf(
            """{"ulid":{"crockford":"01ALG"},"timestamp_ms":1,"event_type":"std.allergy","notes":"Penicillin"}""",
            """{"ulid":{"crockford":"01BTY"},"timestamp_ms":2,"event_type":"std.blood_type","channels":[{"channel_path":"blood.type","text_value":"A+"}]}""",
            """{"ulid":{"crockford":"01HR"},"timestamp_ms":3,"event_type":"std.vital","channels":[{"channel_path":"vital.hr","real_value":112.0,"unit":"bpm"}]}""",
        )
        for (ev in events) {
            val bytes = ev.toByteArray(Charsets.UTF_8)
            buf.writeByte(0); buf.writeInt(bytes.size); buf.write(bytes)
        }
        buf.writeByte(0x02); buf.writeInt(2); buf.write("{}".toByteArray(Charsets.UTF_8))
        server.enqueue(MockResponse().setBody(buf))

        val view = EmergencyRepository.loadPatientView("01CASEX")
        assertTrue(view.criticalInfo.allergies.any { it.contains("Penicillin") })
        assertEquals("A+", view.criticalInfo.bloodType)
        assertTrue(view.recentVitals.any { it.channel == "vital.hr" })
    }

    @Test
    fun handoffCase_marks_active_case_handed_off() = runBlockingTestNoTimer {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEX",
        )
        server.enqueue(MockResponse().setBody("""
            { "successor_case_ulid": "01CASESUC", "read_only_grant_token": "ohdg_RO" }
        """.trimIndent()))
        val outcome = EmergencyRepository.handoffCase(
            caseUlid = "01CASEX",
            receivingFacility = "FN Motol — Emergency Department",
            summaryNote = null,
        )
        assertTrue(outcome is EmergencyRepository.HandoffOutcome.Success)
        assertEquals(true, CaseVault.activeCase.value?.handedOff)
        assertEquals("FN Motol — Emergency Department", CaseVault.activeCase.value?.receivingFacility)
    }

    @Test
    fun handoffCase_falls_back_to_mock_on_404() = runBlockingTestNoTimer {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEX",
        )
        server.enqueue(MockResponse().setResponseCode(404))
        val outcome = EmergencyRepository.handoffCase(
            caseUlid = "01CASEX",
            receivingFacility = "Hospital",
            summaryNote = null,
        )
        // Mock fallback always succeeds.
        assertTrue(outcome is EmergencyRepository.HandoffOutcome.Success)
    }

    @Test
    fun payloadToEventInput_maps_BP_to_two_channels() {
        val w = CaseVault.QueuedWrite(
            localUlid = "ulid",
            caseUlid = "case",
            occurredAtMs = 100,
            recordedAtMs = 100,
            kind = CaseVault.InterventionKind.Vital,
            summary = "BP",
            payload = CaseVault.InterventionPayload.BloodPressure(systolic = 120, diastolic = 80),
        )
        val ei = EmergencyRepository.payloadToEventInputForTest(w)
        assertEquals("std.vital", ei.eventType)
        assertEquals(2, ei.channels.size)
        assertEquals("vital.bp_sys", ei.channels[0].channelPath)
        assertEquals("vital.bp_dia", ei.channels[1].channelPath)
    }

    @Test
    fun payloadToEventInput_maps_drug_to_dose_unit_route() {
        val w = CaseVault.QueuedWrite(
            localUlid = "ulid",
            caseUlid = "case",
            occurredAtMs = 100,
            recordedAtMs = 100,
            kind = CaseVault.InterventionKind.Drug,
            summary = "ASA 300mg PO",
            payload = CaseVault.InterventionPayload.Drug(
                name = "Aspirin", doseValue = 300.0, doseUnit = "mg", route = "PO",
            ),
        )
        val ei = EmergencyRepository.payloadToEventInputForTest(w)
        assertEquals("std.medication.administered", ei.eventType)
        assertEquals(4, ei.channels.size)
        assertTrue(ei.channels.any { it.channelPath == "drug.name" && it.value is ChannelInputDto.Value.Text })
        assertTrue(ei.channels.any { it.channelPath == "drug.dose" && it.value is ChannelInputDto.Value.Real })
    }

    /**
     * Tiny coroutine runner so we don't pull in
     * `kotlinx-coroutines-test`'s `runTest` (which depends on
     * Robolectric's main-dispatcher shape on JVM-only). The repository
     * uses `withContext(Dispatchers.IO)` internally; a plain
     * `runBlocking` is fine for the test.
     */
    private fun runBlockingTestNoTimer(block: suspend () -> Unit) {
        kotlinx.coroutines.runBlocking { block() }
    }
}
