package com.ohd.emergency.data.ohdc

import org.json.JSONArray
import org.json.JSONObject

// =============================================================================
// Connect-Protocol JSON DTOs.
//
// These mirror the Protobuf message shapes from `ohdc.v0` (storage's
// service definitions) using JSON-encoded form per Connect-Protocol spec
// (https://connectrpc.com/docs/protocol/#unary-request).
//
// Hand-rolled because the v0 demo doesn't ship the buf codegen pipeline.
// When storage publishes Kotlin codegen, these become thin wrappers
// around the generated types — the high-level repository surface
// (`PatientView`, `TimelineEntry`, etc.) stays unchanged.
// =============================================================================

// ---- Common ----------------------------------------------------------------

/** Crockford-base32-encoded ULID. The wire form per JSON proto-mapping. */
data class UlidJson(val crockford: String) {
    fun toJson(): JSONObject = JSONObject().put("crockford", crockford)
    companion object {
        fun fromJson(j: JSONObject): UlidJson = UlidJson(j.optString("crockford"))
    }
}

// ---- Filter / query --------------------------------------------------------

/**
 * Mirror of `ohdc.v0.EventFilter`. Only the fields the tablet uses are
 * surfaced; everything else passes through as JSON unchanged.
 */
data class EventFilter(
    val fromMs: Long? = null,
    val toMs: Long? = null,
    val eventTypesIn: List<String> = emptyList(),
    val includeSuperseded: Boolean = true,
    val limit: Long? = null,
    /** Restrict to one case. v0 storage exposes this via the grant scope. */
    val caseUlid: String? = null,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        if (fromMs != null) put("from_ms", fromMs)
        if (toMs != null) put("to_ms", toMs)
        if (eventTypesIn.isNotEmpty()) put("event_types_in", JSONArray(eventTypesIn))
        put("include_superseded", includeSuperseded)
        if (limit != null) put("limit", limit)
        if (caseUlid != null) put("case_ulid", JSONObject().put("crockford", caseUlid))
    }
}

// ---- WhoAmI ----------------------------------------------------------------

data class WhoAmIResult(
    val userUlid: String,
    val tokenKind: String,
    val granteeLabel: String?,
) {
    companion object {
        fun fromJson(j: JSONObject): WhoAmIResult = WhoAmIResult(
            userUlid = j.optJSONObject("user_ulid")?.optString("crockford") ?: "",
            tokenKind = j.optString("token_kind", "unknown"),
            granteeLabel = j.optString("grantee_label").takeIf { it.isNotEmpty() },
        )
    }
}

// ---- Event (read side) -----------------------------------------------------

/**
 * Subset of `ohdc.v0.Event` the tablet uses. Channel values are kept as
 * `String` for simplicity — the OHDC channel-value oneof flattens to a
 * scalar at the JSON level (real_value, int_value, bool_value,
 * text_value, enum_ordinal); we render whichever shows up.
 */
data class EventDto(
    val ulid: String,
    val timestampMs: Long,
    val eventType: String,
    val channels: List<ChannelDto>,
    val notes: String?,
    val source: String?,
    val sourceId: String?,
) {
    companion object {
        fun fromJson(j: JSONObject): EventDto {
            val channelsArr = j.optJSONArray("channels") ?: JSONArray()
            val channels = (0 until channelsArr.length()).map {
                ChannelDto.fromJson(channelsArr.getJSONObject(it))
            }
            return EventDto(
                ulid = j.optJSONObject("ulid")?.optString("crockford") ?: "",
                timestampMs = j.optLong("timestamp_ms", 0L),
                eventType = j.optString("event_type", ""),
                channels = channels,
                notes = j.optString("notes").takeIf { it.isNotEmpty() },
                source = j.optString("source").takeIf { it.isNotEmpty() },
                sourceId = j.optString("source_id").takeIf { it.isNotEmpty() },
            )
        }
    }
}

data class ChannelDto(
    val channelPath: String,
    /** Display-string of whichever scalar oneof was set. */
    val valueDisplay: String,
    /** Numeric value if real/int. */
    val numericValue: Double?,
    val unit: String?,
) {
    companion object {
        fun fromJson(j: JSONObject): ChannelDto {
            // The Connect-Protocol JSON encoding flattens the oneof into
            // sibling fields with a `case` discriminator; we accept both
            // spellings (`real_value` / `realValue`) for resiliency.
            val real = j.opt("real_value") ?: j.opt("realValue")
            val int = j.opt("int_value") ?: j.opt("intValue")
            val bool = j.opt("bool_value") ?: j.opt("boolValue")
            val text = j.opt("text_value") ?: j.opt("textValue")
            val numeric = (real as? Number)?.toDouble() ?: (int as? Number)?.toDouble()
            val display = when {
                real != null -> real.toString()
                int != null -> int.toString()
                bool != null -> bool.toString()
                text != null -> text.toString()
                else -> ""
            }
            return ChannelDto(
                channelPath = j.optString("channel_path", ""),
                valueDisplay = display,
                numericValue = numeric,
                unit = j.optString("unit").takeIf { it.isNotEmpty() },
            )
        }
    }
}

// ---- Event (write side) ----------------------------------------------------

/**
 * Mirror of `ohdc.v0.EventInput`. Builder helpers keep the JSON tedium
 * in one place; the repository constructs these from
 * [com.ohd.emergency.data.CaseVault.InterventionPayload].
 */
data class EventInputDto(
    val timestampMs: Long,
    val eventType: String,
    val channels: List<ChannelInputDto>,
    val notes: String? = null,
    val source: String? = null,
    val sourceId: String? = null,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("timestamp_ms", timestampMs)
        put("event_type", eventType)
        put("channels", JSONArray(channels.map { it.toJson() }))
        if (notes != null) put("notes", notes)
        if (source != null) put("source", source)
        if (sourceId != null) put("source_id", sourceId)
    }
}

/**
 * Channel + value input. The sealed [Value] mirrors OHDC's oneof on
 * `ChannelValue`; encoded as a JSON sibling field per Connect-Protocol.
 */
data class ChannelInputDto(
    val channelPath: String,
    val value: Value,
) {
    sealed interface Value {
        data class Real(val v: Double) : Value
        data class Int(val v: Long) : Value
        data class Bool(val v: Boolean) : Value
        data class Text(val v: String) : Value
        data class Enum(val ordinal: kotlin.Int) : Value
    }

    fun toJson(): JSONObject = JSONObject().apply {
        put("channel_path", channelPath)
        when (val vv = value) {
            is Value.Real -> put("real_value", vv.v)
            is Value.Int -> put("int_value", vv.v)
            is Value.Bool -> put("bool_value", vv.v)
            is Value.Text -> put("text_value", vv.v)
            is Value.Enum -> put("enum_ordinal", vv.ordinal)
        }
    }
}

data class PutEventsResult(
    val results: List<PutOutcome>,
) {
    sealed interface PutOutcome {
        data class Committed(val ulid: String) : PutOutcome
        data class Pending(val ulid: String, val expiresAtMs: Long) : PutOutcome
        data class Error(val code: String, val message: String) : PutOutcome
    }

    companion object {
        fun fromJson(j: JSONObject): PutEventsResult {
            val arr = j.optJSONArray("results") ?: JSONArray()
            val out = (0 until arr.length()).map { i ->
                val r = arr.getJSONObject(i)
                val outcomeObj = r.optJSONObject("outcome") ?: r
                val committed = outcomeObj.optJSONObject("committed")
                val pending = outcomeObj.optJSONObject("pending")
                val error = outcomeObj.optJSONObject("error")
                when {
                    committed != null -> PutOutcome.Committed(
                        ulid = committed.optJSONObject("ulid")?.optString("crockford") ?: "",
                    )
                    pending != null -> PutOutcome.Pending(
                        ulid = pending.optJSONObject("ulid")?.optString("crockford") ?: "",
                        expiresAtMs = pending.optLong("expires_at_ms", 0L),
                    )
                    error != null -> PutOutcome.Error(
                        code = error.optString("code", "unknown"),
                        message = error.optString("message", ""),
                    )
                    else -> PutOutcome.Error("empty_outcome", "outcome oneof was unset")
                }
            }
            return PutEventsResult(out)
        }
    }
}

// ---- Case ------------------------------------------------------------------

data class CaseDto(
    val caseUlid: String,
    val label: String?,
    val status: String,
    val openedAtMs: Long,
    val receivingFacility: String?,
    val predecessorCaseUlid: String?,
) {
    companion object {
        fun fromJson(j: JSONObject): CaseDto = CaseDto(
            caseUlid = j.optJSONObject("case_ulid")?.optString("crockford") ?: "",
            label = j.optString("label").takeIf { it.isNotEmpty() },
            status = j.optString("status", "unknown"),
            openedAtMs = j.optLong("opened_at_ms", 0L),
            receivingFacility = j.optString("receiving_facility").takeIf { it.isNotEmpty() },
            predecessorCaseUlid = j.optJSONObject("predecessor_case_ulid")?.optString("crockford"),
        )
    }
}

// ---- Emergency initiate (relay-private) ------------------------------------

/**
 * Mirror of `relay::server::EmergencyInitiateRequest`. Field names are
 * snake_case to match the relay's `serde` defaults.
 */
data class EmergencyInitiateRequest(
    val rendezvousId: String,
    val patientStoragePubkeyPinHex: String? = null,
    val responderLabel: String? = null,
    val sceneContext: String? = null,
    val operatorLabel: String? = null,
    val sceneLat: Double? = null,
    val sceneLon: Double? = null,
    val sceneAccuracyM: Float? = null,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("rendezvous_id", rendezvousId)
        if (patientStoragePubkeyPinHex != null) put("patient_storage_pubkey_pin_hex", patientStoragePubkeyPinHex)
        if (responderLabel != null) put("responder_label", responderLabel)
        if (sceneContext != null) put("scene_context", sceneContext)
        if (operatorLabel != null) put("operator_label", operatorLabel)
        if (sceneLat != null) put("scene_lat", sceneLat)
        if (sceneLon != null) put("scene_lon", sceneLon)
        if (sceneAccuracyM != null) put("scene_accuracy_m", sceneAccuracyM.toDouble())
    }
}

/**
 * Mirror of `relay::server::EmergencyInitiateResponse`. The relay returns
 * the signed `EmergencyAccessRequest` + a delivery status (`delivered` /
 * `pushed` / `no_token`).
 *
 * The grant token is NOT in this response — it surfaces via a separate
 * channel (relay polling, push, or BLE callback) once the patient
 * approves. We thread the `request_id` through to
 * [OhdcClient.pollEmergencyStatus] for that follow-up.
 */
data class EmergencyInitiateResponse(
    /** From signed_request.request_id; used as the polling key. */
    val requestId: String,
    val deliveryStatus: String,
    /** The full signed payload (returned for audit / display only). */
    val signedRequestJson: String,
) {
    companion object {
        fun fromJson(j: JSONObject): EmergencyInitiateResponse {
            val signed = j.optJSONObject("signed_request") ?: JSONObject()
            return EmergencyInitiateResponse(
                requestId = signed.optString("request_id", ""),
                deliveryStatus = j.optString("delivery_status", "unknown"),
                signedRequestJson = signed.toString(),
            )
        }
    }
}

/**
 * Mirror of the relay's emergency status poll. Provisional shape — the
 * relay endpoint isn't yet wired (see `relay/STATUS.md` "What's stubbed
 * / TBD"); when it lands the JSON keys are expected to be these. If the
 * relay returns a different shape, this DTO is the one place to update.
 */
data class EmergencyStatusDto(
    val state: String,
    val caseUlid: String?,
    val grantToken: String?,
    val patientLabel: String?,
    val rejectedReason: String?,
    val expiresAtMs: Long?,
) {
    companion object {
        fun fromJson(j: JSONObject): EmergencyStatusDto = EmergencyStatusDto(
            state = j.optString("state", "unknown"),
            caseUlid = j.optString("case_ulid").takeIf { it.isNotEmpty() },
            grantToken = j.optString("grant_token").takeIf { it.isNotEmpty() },
            patientLabel = j.optString("patient_label").takeIf { it.isNotEmpty() },
            rejectedReason = j.optString("rejected_reason").takeIf { it.isNotEmpty() },
            expiresAtMs = j.optLong("expires_at_ms").takeIf { it > 0L },
        )
    }
}

// ---- Handoff (relay-private) -----------------------------------------------

data class HandoffResponseDto(
    val successorCaseUlid: String,
    val readOnlyGrantToken: String,
) {
    companion object {
        fun fromJson(j: JSONObject): HandoffResponseDto = HandoffResponseDto(
            successorCaseUlid = j.optString("successor_case_ulid", ""),
            readOnlyGrantToken = j.optString("read_only_grant_token", ""),
        )
    }
}
