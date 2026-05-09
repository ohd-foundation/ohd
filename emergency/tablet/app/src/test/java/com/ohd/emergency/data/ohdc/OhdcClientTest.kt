package com.ohd.emergency.data.ohdc

import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.Buffer
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * Round-trip tests for [OhdcClient] against a [MockWebServer].
 *
 * These cover the wire shape:
 *  - Connect-Protocol unary (PutEvents, GetCase, ListCases, WhoAmI)
 *  - Connect-Protocol server-streaming (QueryEvents)
 *  - Relay-private REST (emergency/initiate, emergency/handoff)
 *  - Auth header selection (operator OIDC vs grant token)
 *  - Error envelope decoding (Connect error JSON / HTTP error)
 */
class OhdcClientTest {

    private lateinit var server: MockWebServer
    private lateinit var client: OhdcClient

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
        client = OhdcClient(
            baseUrl = server.url("").toString().trimEnd('/'),
            operatorBearerProvider = { "operator_bearer_xyz" },
            grantTokenProvider = { "ohdg_GRANT_abc" },
        )
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    @Test
    fun whoAmI_unary_round_trip_uses_grant_bearer() {
        server.enqueue(MockResponse().setBody("""
            { "user_ulid": { "crockford": "01PATIENTULID" },
              "token_kind": "grant",
              "grantee_label": "EMS Prague Region — Crew 42" }
        """.trimIndent()))

        val res = client.whoAmI().getOrThrow()

        assertEquals("01PATIENTULID", res.userUlid)
        assertEquals("grant", res.tokenKind)
        assertEquals("EMS Prague Region — Crew 42", res.granteeLabel)

        val req = server.takeRequest()
        assertEquals("POST", req.method)
        assertEquals("/ohdc.v0.OhdcService/WhoAmI", req.path)
        assertEquals("Bearer ohdg_GRANT_abc", req.getHeader("Authorization"))
        assertEquals("1", req.getHeader("Connect-Protocol-Version"))
    }

    @Test
    fun queryEvents_streaming_decodes_enveloped_frames() {
        // Build a Connect server-stream body: two message frames, then
        // an end-stream frame. Each: [flags=byte][length=BE i32][JSON].
        val buf = Buffer()
        val ev1 = """{"ulid":{"crockford":"01EVTONE"},"timestamp_ms":1700000000000,"event_type":"std.vital","channels":[{"channel_path":"vital.hr","real_value":112.0}]}"""
        val ev2 = """{"ulid":{"crockford":"01EVTTWO"},"timestamp_ms":1700000060000,"event_type":"std.observation","notes":"alert"}"""
        writeFrame(buf, flags = 0, payload = ev1)
        writeFrame(buf, flags = 0, payload = ev2)
        writeFrame(buf, flags = 0x02, payload = "{}") // EndStreamResponse, no error

        server.enqueue(MockResponse().setBody(buf))

        val out = client.queryEvents(EventFilter(limit = 10)).getOrThrow()
        assertEquals(2, out.size)
        assertEquals("01EVTONE", out[0].ulid)
        assertEquals(112.0, out[0].channels.first().numericValue!!, 0.001)
        assertEquals("alert", out[1].notes)
    }

    @Test
    fun queryEvents_end_stream_error_is_propagated() {
        val buf = Buffer()
        writeFrame(buf, flags = 0x02, payload = """{"error":{"code":"permission_denied","message":"grant lacks scope"}}""")
        server.enqueue(MockResponse().setBody(buf))

        val res = client.queryEvents(EventFilter())
        assertTrue("expected failure", res.isFailure)
        val err = res.exceptionOrNull() as? OhdcException
        assertNotNull(err)
        assertEquals("grant lacks scope", err!!.message)
    }

    @Test
    fun putEvents_unary_decodes_outcomes() {
        server.enqueue(MockResponse().setBody("""
            { "results": [
                { "outcome": { "committed": { "ulid": { "crockford": "01EVTCOMMIT" } } } },
                { "outcome": { "pending":   { "ulid": { "crockford": "01EVTPENDING" }, "expires_at_ms": 9999 } } },
                { "outcome": { "error":     { "code": "channel_unknown", "message": "vital.foo not in template" } } }
            ] }
        """.trimIndent()))

        val res = client.putEvents(events = listOf(
            EventInputDto(
                timestampMs = 1L,
                eventType = "std.vital",
                channels = listOf(ChannelInputDto("vital.hr", ChannelInputDto.Value.Real(112.0))),
            ),
        )).getOrThrow()
        assertEquals(3, res.results.size)
        assertTrue(res.results[0] is PutEventsResult.PutOutcome.Committed)
        assertEquals("01EVTCOMMIT", (res.results[0] as PutEventsResult.PutOutcome.Committed).ulid)
        assertTrue(res.results[1] is PutEventsResult.PutOutcome.Pending)
        assertTrue(res.results[2] is PutEventsResult.PutOutcome.Error)
    }

    @Test
    fun unary_error_envelope_decoded() {
        server.enqueue(MockResponse().setResponseCode(403).setBody("""
            { "code": "permission_denied", "message": "grant token expired" }
        """.trimIndent()))
        val res = client.whoAmI()
        val err = res.exceptionOrNull() as? OhdcException
        assertNotNull(err)
        assertEquals("permission_denied", err!!.connectCode)
        assertEquals(403, err.code)
    }

    @Test
    fun emergencyInitiate_sends_operator_bearer_and_decodes_response() {
        server.enqueue(MockResponse().setBody("""
            { "signed_request": { "request_id": "abc123", "issued_at_ms": 1, "cert_chain_pem": [] },
              "delivery_status": "delivered" }
        """.trimIndent()))

        val res = client.emergencyInitiate(EmergencyInitiateRequest(
            rendezvousId = "rdv_xyz",
            responderLabel = "Officer Novák",
            sceneContext = "Václavské nám.",
        )).getOrThrow()
        assertEquals("abc123", res.requestId)
        assertEquals("delivered", res.deliveryStatus)

        val req = server.takeRequest()
        assertEquals("POST", req.method)
        assertEquals("/v1/emergency/initiate", req.path)
        // Operator OIDC bearer, NOT the grant token.
        assertEquals("Bearer operator_bearer_xyz", req.getHeader("Authorization"))
        val body = JSONObject(req.body.readUtf8())
        assertEquals("rdv_xyz", body.getString("rendezvous_id"))
        assertEquals("Officer Novák", body.getString("responder_label"))
        assertEquals("Václavské nám.", body.getString("scene_context"))
    }

    @Test
    fun emergencyInitiate_404_returns_failure() {
        server.enqueue(MockResponse().setResponseCode(404).setBody("not implemented"))
        val res = client.emergencyInitiate(EmergencyInitiateRequest(rendezvousId = "rdv"))
        val err = res.exceptionOrNull() as? OhdcException
        assertNotNull(err)
        assertEquals(404, err!!.code)
    }

    @Test
    fun pollEmergencyStatus_decodes_state_variants() {
        server.enqueue(MockResponse().setBody("""
            { "state": "approved",
              "case_ulid": "01CASEXYZ",
              "grant_token": "ohdg_NEW",
              "patient_label": "Patient A" }
        """.trimIndent()))
        val res = client.pollEmergencyStatus("abc").getOrThrow()
        assertEquals("approved", res.state)
        assertEquals("01CASEXYZ", res.caseUlid)
        assertEquals("ohdg_NEW", res.grantToken)

        val req = server.takeRequest()
        assertEquals("GET", req.method)
        assertEquals("/v1/emergency/status/abc", req.path)
    }

    @Test
    fun emergencyHandoff_round_trip() {
        server.enqueue(MockResponse().setBody("""
            { "successor_case_ulid": "01CASESUC",
              "read_only_grant_token": "ohdg_RO" }
        """.trimIndent()))
        val res = client.emergencyHandoff(
            caseUlid = "01CASEPRIOR",
            receivingFacility = "FN Motol — Emergency Department",
            summaryNote = "Brief",
        ).getOrThrow()
        assertEquals("01CASESUC", res.successorCaseUlid)
        assertEquals("ohdg_RO", res.readOnlyGrantToken)

        val req = server.takeRequest()
        assertEquals("/v1/emergency/handoff", req.path)
        val body = JSONObject(req.body.readUtf8())
        assertEquals("01CASEPRIOR", body.getString("case_ulid"))
        assertEquals("FN Motol — Emergency Department", body.getString("receiving_facility"))
        assertEquals("Brief", body.getString("summary_note"))
    }

    @Test
    fun listCases_decodes_case_array() {
        server.enqueue(MockResponse().setBody("""
            { "cases": [
                { "case_ulid": { "crockford": "01CASEAAA" }, "status": "open", "opened_at_ms": 1 },
                { "case_ulid": { "crockford": "01CASEBBB" }, "status": "closed", "opened_at_ms": 2 }
            ] }
        """.trimIndent()))
        val out = client.listCases(includeClosed = true).getOrThrow()
        assertEquals(2, out.size)
        assertEquals("01CASEAAA", out[0].caseUlid)
        assertEquals("open", out[0].status)
        assertEquals("closed", out[1].status)
    }

    @Test
    fun client_omits_authorization_when_provider_returns_null() {
        val noAuthClient = OhdcClient(
            baseUrl = server.url("").toString().trimEnd('/'),
            operatorBearerProvider = { null },
            grantTokenProvider = { null },
        )
        server.enqueue(MockResponse().setBody("""{"user_ulid":{"crockford":"X"},"token_kind":"none"}"""))
        noAuthClient.whoAmI().getOrThrow()
        val req = server.takeRequest()
        assertNull(req.getHeader("Authorization"))
    }

    /** Helper: write one Connect-Protocol enveloped frame. */
    private fun writeFrame(buf: Buffer, flags: Int, payload: String) {
        val bytes = payload.toByteArray(Charsets.UTF_8)
        buf.writeByte(flags)
        buf.writeInt(bytes.size)
        buf.write(bytes)
    }

    @Test
    fun event_filter_serializes_known_fields() {
        // Sanity check that EventFilter.toJson() emits the fields our
        // tests + repository expect.
        val filter = EventFilter(
            fromMs = 1L, toMs = 2L,
            eventTypesIn = listOf("std.vital", "std.observation"),
            limit = 50L,
            caseUlid = "01CASEX",
        )
        val j = filter.toJson()
        assertEquals(1L, j.getLong("from_ms"))
        assertEquals(50L, j.getLong("limit"))
        assertEquals("01CASEX", j.getJSONObject("case_ulid").getString("crockford"))
        val types = j.getJSONArray("event_types_in")
        assertEquals(JSONArray::class.java, types.javaClass)
        assertEquals(2, types.length())
    }
}
