package com.ohd.emergency.data.ohdc

import android.util.Log
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.util.concurrent.TimeUnit

/**
 * OHDC client over HTTP — talks Connect-RPC unary/JSON to the storage
 * service at the operator's relay base URL, using either:
 *  - the operator's OIDC bearer (for relay-internal endpoints), OR
 *  - the per-case grant token returned by `/v1/emergency/initiate`.
 *
 * # Wire decision: hand-rolled OkHttp + Connect-Protocol JSON
 *
 * Two options were on the table:
 *   A. **Connect-Kotlin** (https://github.com/connectrpc/connect-kotlin):
 *      generated stubs from buf workflow, AGP plugin, full proto codegen
 *      pipeline.
 *   B. **Hand-rolled OkHttp + JSON**: tiny client speaking the
 *      Connect-Protocol unary wire by hand (POST + JSON body +
 *      `connect-protocol-version: 1` header).
 *
 * **Picked B** for these reasons:
 *
 *  1. The relay's `/v1/emergency/initiate` is **plain JSON over HTTP**,
 *     not Connect-RPC at all. So even with Connect-Kotlin we'd need a
 *     hand-rolled HTTP path for the relay's emergency surface anyway.
 *     Doubling up on HTTP plumbing across two libraries is friction.
 *  2. Connect-Kotlin's buf gen pipeline + AGP plugin would force a
 *     proto-generation step into `BUILD.md` that the v0 demo doesn't
 *     need (the proto stubs aren't shipping from emergency/tablet's
 *     codegen — see BUILD.md "OHDC client").
 *  3. The OHDC unary RPCs we need (`WhoAmI`, `QueryEvents`, `PutEvents`,
 *     `GetCase`, `ListCases`) are tiny payloads. Connect-Protocol JSON is
 *     specified at https://connectrpc.com/docs/protocol/ — POST to
 *     `/<package>.<Service>/<Method>`, JSON body, JSON response.
 *  4. Connect-Web (the sibling Care app) speaks the same wire; this
 *     client mirrors that pattern in Kotlin.
 *  5. **Streaming RPCs** (`QueryEvents` returns `stream Event`) are
 *     handled by Connect-Protocol's "JSON-encoded enveloped stream"
 *     framing — see [streamingPost]. Implementation matches the spec at
 *     https://connectrpc.com/docs/protocol/#streaming-rpcs.
 *
 * Once the storage component publishes Kotlin codegen drops (binary
 * protobuf), this client gains a sibling `OhdcBinaryClient` wired to
 * those stubs. The high-level repository API stays unchanged.
 *
 * # Tunneling through the operator's relay
 *
 * The tablet does NOT talk directly to the patient's storage daemon.
 * Per `STATUS.md` "Three wire surfaces" + `BUILD.md`:
 *
 *     OHDC HTTP client used for everything that's not the local cache —
 *     break-glass initiation, PutEvents for interventions, QueryEvents
 *     for the patient view, HandoffCase.
 *
 * The operator's relay proxies OHDC calls to the patient's storage over
 * the relay-mediated tunnel. The tablet therefore POSTs OHDC requests to
 * the relay's HTTP base URL; the relay forwards. Auth is the case grant
 * token in the `Authorization: Bearer …` header (the relay validates
 * grant scope before forwarding).
 *
 * # Bearer selection
 *
 * Two bearers come into play depending on the call:
 *
 *  - **Operator OIDC bearer** (`OperatorSession.bearer(ctx)`): the
 *    relay-private endpoints (`/v1/emergency/initiate`,
 *    `/v1/emergency/handoff`, `/v1/auth/info`) gate on this. The
 *    operator is "the responder logging in for their shift" — the relay
 *    knows the operator and is willing to sign emergency requests for them.
 *  - **Case grant token** (`CaseVault.activeCase.value.grantToken`): the
 *    OHDC RPCs (`PutEvents`, `QueryEvents`) gate on this. Issued by the
 *    patient phone after break-glass; scoped to one case; the relay
 *    forwards bytes opaquely.
 *
 * Each public method below documents which bearer it uses.
 */
class OhdcClient(
    /** Base URL of the operator's relay, e.g. `https://relay.ems-prague.cz`. */
    private val baseUrl: String,
    /** Lazy operator-OIDC bearer accessor (refreshed token after silent refresh). */
    private val operatorBearerProvider: () -> String?,
    /** Lazy active-case grant accessor (cleared on case close). */
    private val grantTokenProvider: () -> String?,
    /** OkHttp client with HTTP/2 + reasonable timeouts. */
    private val http: OkHttpClient = defaultHttp(),
) {

    companion object {
        private const val TAG = "OhdEmergency.OhdcClient"
        private const val CONNECT_PROTOCOL_VERSION = "1"
        private val JSON_MEDIA = "application/json".toMediaType()

        /** Pre-flight defaults; tunable per deployment. */
        fun defaultHttp(): OkHttpClient = OkHttpClient.Builder()
            // HTTP/2 first for OHDC unary multiplexing; H1.1 fallback for
            // operator relays without HTTP/2.
            .protocols(listOf(Protocol.HTTP_2, Protocol.HTTP_1_1))
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .callTimeout(60, TimeUnit.SECONDS)
            .retryOnConnectionFailure(true)
            .build()
    }

    // -----------------------------------------------------------------
    // Connect-Protocol unary path.
    // -----------------------------------------------------------------

    /**
     * Issue a Connect-Protocol JSON unary call.
     *
     * Wire: `POST {baseUrl}/{servicePath}/{method}` with JSON body and
     * `Content-Type: application/json` + `Connect-Protocol-Version: 1`.
     * Response is a JSON-encoded message; on error, body is a Connect
     * `Error` envelope `{ "code": "...", "message": "..." }`.
     *
     * `servicePath` is the dotted Protobuf service name without leading
     * slash, e.g. `ohdc.v0.OhdcService`. We prefix `/` here so callers
     * pass clean strings.
     *
     * `useGrantToken`: if true, sends the case grant; if false, sends
     * the operator OIDC bearer.
     */
    internal fun unary(
        servicePath: String,
        method: String,
        bodyJson: String,
        useGrantToken: Boolean,
    ): Result<String> = runCatching {
        val url = "${baseUrl.trimEnd('/')}/$servicePath/$method"
        val bearer = if (useGrantToken) grantTokenProvider() else operatorBearerProvider()
        val req = Request.Builder()
            .url(url)
            .post(bodyJson.toRequestBody(JSON_MEDIA))
            .header("Content-Type", "application/json")
            .header("Connect-Protocol-Version", CONNECT_PROTOCOL_VERSION)
            .also { b -> if (bearer != null) b.header("Authorization", "Bearer $bearer") }
            .build()
        http.newCall(req).execute().use { resp -> readUnaryResponse(resp) }
    }.onFailure { Log.w(TAG, "unary $servicePath/$method failed", it) }

    /**
     * Streaming unary — Connect-Protocol's "server-streaming" framing.
     *
     * Each enveloped frame is `[1 byte flags][4 bytes BE length][N JSON bytes]`.
     * The trailing frame's `flags` has bit 1 (0x02) set and the body is
     * an `EndStreamResponse` JSON `{ "error": ?, "metadata": ?... }`.
     *
     * Returns the message JSON payloads in order; on error throws.
     */
    internal fun streamingPost(
        servicePath: String,
        method: String,
        bodyJson: String,
        useGrantToken: Boolean,
    ): Result<List<String>> = runCatching {
        val url = "${baseUrl.trimEnd('/')}/$servicePath/$method"
        val bearer = if (useGrantToken) grantTokenProvider() else operatorBearerProvider()
        val req = Request.Builder()
            .url(url)
            .post(bodyJson.toRequestBody("application/connect+json".toMediaType()))
            .header("Content-Type", "application/connect+json")
            .header("Connect-Protocol-Version", CONNECT_PROTOCOL_VERSION)
            .also { b -> if (bearer != null) b.header("Authorization", "Bearer $bearer") }
            .build()
        http.newCall(req).execute().use { resp ->
            if (!resp.isSuccessful) {
                throw OhdcException("HTTP ${resp.code}", resp.code, resp.body?.string().orEmpty())
            }
            val src = resp.body?.source() ?: throw OhdcException("empty body", resp.code, "")
            val out = mutableListOf<String>()
            while (!src.exhausted()) {
                val flags = src.readByte().toInt() and 0xFF
                val length = src.readInt() // BE
                val payload = src.readByteArray(length.toLong()).toString(Charsets.UTF_8)
                if (flags and 0x02 != 0) {
                    // EndStreamResponse — check for error field.
                    val obj = runCatching { JSONObject(payload) }.getOrNull()
                    val err = obj?.optJSONObject("error")
                    if (err != null) {
                        throw OhdcException(
                            err.optString("message", "unknown"),
                            resp.code,
                            err.optString("code", ""),
                        )
                    }
                    break
                } else {
                    out.add(payload)
                }
            }
            out
        }
    }.onFailure { Log.w(TAG, "stream $servicePath/$method failed", it) }

    private fun readUnaryResponse(resp: Response): String {
        val body = resp.body?.string().orEmpty()
        if (!resp.isSuccessful) {
            // Connect error envelope: { "code": "permission_denied", "message": "..." }
            val (code, message) = runCatching {
                val obj = JSONObject(body)
                obj.optString("code", "unknown") to obj.optString("message", body)
            }.getOrDefault("http_${resp.code}" to body)
            throw OhdcException(message, resp.code, code)
        }
        return body
    }

    // -----------------------------------------------------------------
    // Public RPC façade.
    //
    // These wrap the OHDC service's unary + streaming methods at the
    // ohdc.v0 namespace. JSON shape matches the proto generated form
    // (Connect-Protocol JSON encoding spec).
    // -----------------------------------------------------------------

    /**
     * `OhdcService.WhoAmI` — diagnostic. Returns the grantee label +
     * effective grant. Useful as a smoke test that the case grant token
     * was accepted by the storage.
     */
    fun whoAmI(): Result<WhoAmIResult> = unary(
        servicePath = "ohdc.v0.OhdcService",
        method = "WhoAmI",
        bodyJson = "{}",
        useGrantToken = true,
    ).map { WhoAmIResult.fromJson(JSONObject(it)) }

    /**
     * `OhdcService.QueryEvents` — server-streaming. Returns every event
     * matching the filter under the case grant.
     *
     * The patient view filter narrows by `event_types_in` (allergies,
     * medications, vitals, observations, advance directives) — the
     * emergency-template's channel set per the patient's emergency profile.
     */
    fun queryEvents(filter: EventFilter): Result<List<EventDto>> = streamingPost(
        servicePath = "ohdc.v0.OhdcService",
        method = "QueryEvents",
        bodyJson = JSONObject().apply {
            put("filter", filter.toJson())
        }.toString(),
        useGrantToken = true,
    ).map { frames -> frames.map { EventDto.fromJson(JSONObject(it)) } }

    /**
     * `OhdcService.PutEvents` — unary. Submits one or more events under
     * the case grant. Used for intervention writes (vitals, drugs,
     * observations, notes).
     */
    fun putEvents(events: List<EventInputDto>, atomic: Boolean = false): Result<PutEventsResult> = unary(
        servicePath = "ohdc.v0.OhdcService",
        method = "PutEvents",
        bodyJson = JSONObject().apply {
            put("events", JSONArray(events.map { it.toJson() }))
            put("atomic", atomic)
        }.toString(),
        useGrantToken = true,
    ).map { PutEventsResult.fromJson(JSONObject(it)) }

    /**
     * `OhdcService.GetCase` — unary. Returns case metadata (status,
     * predecessor, receiving authority, etc.).
     */
    fun getCase(caseUlid: String): Result<CaseDto> = unary(
        servicePath = "ohdc.v0.OhdcService",
        method = "GetCase",
        bodyJson = JSONObject().apply {
            put("case_ulid", JSONObject().put("crockford", caseUlid))
        }.toString(),
        useGrantToken = true,
    ).map { CaseDto.fromJson(JSONObject(it).optJSONObject("case") ?: JSONObject(it)) }

    /**
     * `OhdcService.ListCases` — unary. Returns all open + recently-closed
     * cases visible under the current grant. Tablet uses this for the
     * resume-active-case banner on Discovery (cross-checks the
     * CaseVault).
     */
    fun listCases(includeClosed: Boolean = false): Result<List<CaseDto>> = unary(
        servicePath = "ohdc.v0.OhdcService",
        method = "ListCases",
        bodyJson = JSONObject().apply {
            put("include_closed", includeClosed)
        }.toString(),
        useGrantToken = true,
    ).map { json ->
        val arr = JSONObject(json).optJSONArray("cases") ?: JSONArray()
        (0 until arr.length()).map { CaseDto.fromJson(arr.getJSONObject(it)) }
    }

    // -----------------------------------------------------------------
    // Relay-private endpoints (NOT OHDC RPC; plain REST/JSON over HTTP).
    //
    // Use the operator OIDC bearer.
    // -----------------------------------------------------------------

    /**
     * `POST /v1/emergency/initiate` — relay-private.
     *
     * Wire shape mirrors `relay/src/server.rs::handle_emergency_initiate`:
     *  request:  `EmergencyInitiateRequest` (rendezvous_id, scene
     *            context, optional pin / labels / coords)
     *  response: `EmergencyInitiateResponse` (signed_request,
     *            delivery_status: "delivered" | "pushed" | "no_token")
     *
     * The relay signs the request with its Fulcio-issued leaf cert and
     * pushes a wake to the patient's device. The patient's storage then
     * delivers the signed payload to its OHD Connect app, which renders
     * the break-glass dialog.
     *
     * The grant token is NOT returned by this endpoint — it arrives via
     * a separate channel after the patient approves. v1 polls the relay
     * with [pollEmergencyStatus] until a grant token surfaces (or the
     * timeout fires).
     */
    fun emergencyInitiate(req: EmergencyInitiateRequest): Result<EmergencyInitiateResponse> = runCatching {
        val url = "${baseUrl.trimEnd('/')}/v1/emergency/initiate"
        val bearer = operatorBearerProvider()
            ?: throw OhdcException("missing operator bearer", 0, "no_bearer")
        val httpReq = Request.Builder()
            .url(url)
            .post(req.toJson().toString().toRequestBody(JSON_MEDIA))
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer $bearer")
            .build()
        http.newCall(httpReq).execute().use { resp ->
            val body = resp.body?.string().orEmpty()
            if (!resp.isSuccessful) {
                throw OhdcException("HTTP ${resp.code}", resp.code, body)
            }
            EmergencyInitiateResponse.fromJson(JSONObject(body))
        }
    }.onFailure { Log.w(TAG, "/v1/emergency/initiate failed", it) }

    /**
     * Long-poll for the patient's response after [emergencyInitiate].
     *
     * v1 wire (provisional, mirrors what the relay's emergency push-wake
     * delivery loop would expose):
     *
     *     GET /v1/emergency/status/{request_id}
     *     Authorization: Bearer <operator OIDC bearer>
     *
     *     200 →
     *     { "state": "waiting" | "approved" | "rejected" | "auto_granted" | "timed_out",
     *       "case_ulid": "...",
     *       "grant_token": "ohdg_...",   (when state=approved | auto_granted)
     *       "patient_label": "...",
     *       "rejected_reason": "...",
     *       "expires_at_ms": ...
     *     }
     *
     * The relay-side endpoint is not yet wired (see relay STATUS.md
     * "What's stubbed / TBD"); the tablet calls it anyway and falls back
     * to the timeout-default-allow stub if HTTP 404. Once the relay
     * lands the endpoint, the tablet starts honouring real responses
     * without a code change.
     */
    fun pollEmergencyStatus(requestId: String): Result<EmergencyStatusDto> = runCatching {
        val url = "${baseUrl.trimEnd('/')}/v1/emergency/status/$requestId"
        val bearer = operatorBearerProvider()
            ?: throw OhdcException("missing operator bearer", 0, "no_bearer")
        val httpReq = Request.Builder()
            .url(url)
            .get()
            .header("Authorization", "Bearer $bearer")
            .build()
        http.newCall(httpReq).execute().use { resp ->
            val body = resp.body?.string().orEmpty()
            if (!resp.isSuccessful) {
                throw OhdcException("HTTP ${resp.code}", resp.code, body)
            }
            EmergencyStatusDto.fromJson(JSONObject(body))
        }
    }

    /**
     * `POST /v1/emergency/handoff` — relay-private.
     *
     * Wire shape (provisional; mirrors the SPEC's "Handoff endpoint that
     * opens a successor case under the receiving operator's authority"):
     *
     *     POST {operator_relay}/v1/emergency/handoff
     *     Authorization: Bearer <operator OIDC bearer>
     *     Content-Type: application/json
     *     {
     *       "case_ulid": "...",
     *       "receiving_facility": "FN Motol — Emergency Department",
     *       "summary_note": "..."   (optional)
     *     }
     *
     *     200 →
     *     { "successor_case_ulid": "...",
     *       "read_only_grant_token": "..." }
     *
     * Endpoint is not yet wired on the relay; the call falls through to
     * a 404 + caller fallback (mock OK) when absent.
     */
    fun emergencyHandoff(
        caseUlid: String,
        receivingFacility: String,
        summaryNote: String?,
    ): Result<HandoffResponseDto> = runCatching {
        val url = "${baseUrl.trimEnd('/')}/v1/emergency/handoff"
        val bearer = operatorBearerProvider()
            ?: throw OhdcException("missing operator bearer", 0, "no_bearer")
        val body = JSONObject().apply {
            put("case_ulid", caseUlid)
            put("receiving_facility", receivingFacility)
            if (summaryNote != null) put("summary_note", summaryNote)
        }.toString()
        val httpReq = Request.Builder()
            .url(url)
            .post(body.toRequestBody(JSON_MEDIA))
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer $bearer")
            .build()
        http.newCall(httpReq).execute().use { resp ->
            val rb = resp.body?.string().orEmpty()
            if (!resp.isSuccessful) {
                throw OhdcException("HTTP ${resp.code}", resp.code, rb)
            }
            HandoffResponseDto.fromJson(JSONObject(rb))
        }
    }
}

/**
 * Connect-Protocol error envelope, exposed as a Kotlin exception.
 *
 *  - [code] — HTTP status (or 0 for transport-level failures)
 *  - [connectCode] — Connect-RPC code string ("permission_denied",
 *    "unauthenticated", etc.) or HTTP-status-prefixed when not present
 */
class OhdcException(
    message: String,
    val code: Int,
    val connectCode: String,
) : IOException(message)
