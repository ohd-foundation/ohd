package com.ohd.connect.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.io.InputStream
import java.io.InputStreamReader
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/**
 * Preview of a JSONL file. `firstRecords` is up to five flattened records
 * keyed by dotted JSON path; `paths` is the union of paths discovered across
 * the first 100 lines (capped to keep big files responsive).
 */
data class JsonlPreview(
    val paths: List<String>,
    val firstRecords: List<Map<String, Any?>>,
    val totalRecordEstimate: Int?,
)

/** Per-path mapping decided by the user in the importer UI. */
data class JsonlMapping(
    val path: String,
    val mode: Mode,
) {
    sealed interface Mode {
        data object Skip : Mode
        data object Timestamp : Mode
        data class Channel(
            val channelPath: String,
            val type: ChannelType,
            val unit: String?,
        ) : Mode
    }
}

/** Scalar channel type the user picks for a mapped JSON path. */
enum class ChannelType { Real, Int, Bool, Text }

/** Result of [JsonlImporter.import]. */
data class ImportSummary(val emitted: Int, val errors: Int, val firstError: String?)

/**
 * Generic JSONL → OHD events importer. Pure parsing + storage emission, no UI.
 * One JSON object per input line; nested objects flatten to dotted paths and
 * arrays use `[i]` indices (e.g. `nested.value`, `arr[0]`).
 */
object JsonlImporter {

    sealed interface TimestampMode {
        data object NowForAllRecords : TimestampMode
        data class FromPath(val path: String) : TimestampMode
    }

    /** Scan up to 100 lines and return the union of paths plus 5 sample records. */
    suspend fun preview(stream: InputStream): Result<JsonlPreview> = withContext(Dispatchers.IO) {
        runCatching {
            val reader = BufferedReader(InputStreamReader(stream, Charsets.UTF_8))
            val samples = mutableListOf<Map<String, Any?>>()
            val pathOrder = LinkedHashSet<String>()
            var seen = 0
            reader.useLines { lines ->
                for (raw in lines) {
                    val line = raw.trim()
                    if (line.isEmpty()) continue
                    if (seen >= 100) break
                    val parsed = runCatching { JSONObject(line) }.getOrNull() ?: continue
                    val flat = LinkedHashMap<String, Any?>()
                    flatten(parsed, prefix = "", out = flat)
                    pathOrder.addAll(flat.keys)
                    if (samples.size < 5) samples += flat
                    seen++
                }
            }
            JsonlPreview(
                paths = pathOrder.toList(),
                firstRecords = samples,
                totalRecordEstimate = if (seen < 100) seen else null,
            )
        }
    }

    /**
     * Stream every line and emit one OHD event per record. Errors are
     * counted, not thrown — caller surfaces them via [ImportSummary].
     */
    suspend fun import(
        stream: InputStream,
        eventType: String,
        timestampMode: TimestampMode,
        mappings: List<JsonlMapping>,
    ): ImportSummary = withContext(Dispatchers.IO) {
        val reader = BufferedReader(InputStreamReader(stream, Charsets.UTF_8))
        val channelMappings = mappings.mapNotNull { m ->
            (m.mode as? JsonlMapping.Mode.Channel)?.let { m.path to it }
        }
        val timestampPath: String? = when (timestampMode) {
            is TimestampMode.FromPath -> timestampMode.path
            is TimestampMode.NowForAllRecords -> null
        } ?: mappings.firstOrNull { it.mode is JsonlMapping.Mode.Timestamp }?.path

        var emitted = 0
        var errors = 0
        var firstError: String? = null

        reader.useLines { lines ->
            for (raw in lines) {
                val line = raw.trim()
                if (line.isEmpty()) continue
                try {
                    val obj = JSONObject(line)
                    val flat = LinkedHashMap<String, Any?>()
                    flatten(obj, prefix = "", out = flat)

                    val tsMs: Long = timestampPath?.let { p ->
                        parseTimestamp(flat[p])
                    } ?: System.currentTimeMillis()

                    val channels = channelMappings.mapNotNull { (path, ch) ->
                        val raw = flat[path] ?: return@mapNotNull null
                        coerce(raw, ch.type)?.let { scalar ->
                            EventChannelInput(path = ch.channelPath, scalar = scalar)
                        }
                    }
                    if (channels.isEmpty()) {
                        errors++
                        if (firstError == null) firstError = "no channels mapped on a record"
                        continue
                    }

                    val outcome = StorageRepository.putEvent(
                        EventInput(
                            timestampMs = tsMs,
                            eventType = eventType,
                            channels = channels,
                            source = "import:jsonl",
                        ),
                    )
                    outcome
                        .onSuccess { res ->
                            when (res) {
                                is PutEventOutcome.Committed, is PutEventOutcome.Pending -> emitted++
                                is PutEventOutcome.Error -> {
                                    errors++
                                    if (firstError == null) firstError = "${res.code}: ${res.message}"
                                }
                            }
                        }
                        .onFailure { e ->
                            errors++
                            if (firstError == null) firstError = e.message ?: e::class.simpleName.orEmpty()
                        }
                } catch (e: Exception) {
                    errors++
                    if (firstError == null) firstError = e.message ?: e::class.simpleName.orEmpty()
                }
            }
        }
        ImportSummary(emitted = emitted, errors = errors, firstError = firstError)
    }

    // -------------------------------------------------------------------------
    // Internals
    // -------------------------------------------------------------------------

    /**
     * Walk a JSONObject/JSONArray tree, emitting one entry per leaf into [out]
     * keyed by the dotted/indexed JSON path. Nested objects use `.` between
     * keys; arrays use `[i]`. Leaf values are unboxed to Kotlin types (String,
     * Long, Double, Boolean) or `null` for `JSONObject.NULL`.
     */
    private fun flatten(node: Any?, prefix: String, out: MutableMap<String, Any?>) {
        when (node) {
            is JSONObject -> {
                val keys = node.keys()
                while (keys.hasNext()) {
                    val k = keys.next()
                    val child = node.opt(k)
                    val nextPrefix = if (prefix.isEmpty()) k else "$prefix.$k"
                    flatten(child, nextPrefix, out)
                }
            }
            is JSONArray -> {
                for (i in 0 until node.length()) {
                    val child = node.opt(i)
                    val nextPrefix = "$prefix[$i]"
                    flatten(child, nextPrefix, out)
                }
            }
            JSONObject.NULL, null -> out[prefix] = null
            else -> out[prefix] = node
        }
    }

    /** Coerce a raw flattened JSON value into the channel scalar the user picked. */
    private fun coerce(raw: Any?, type: ChannelType): OhdScalar? {
        if (raw == null) return null
        return when (type) {
            ChannelType.Real -> when (raw) {
                is Number -> OhdScalar.Real(raw.toDouble())
                is Boolean -> OhdScalar.Real(if (raw) 1.0 else 0.0)
                is String -> raw.toDoubleOrNull()?.let { OhdScalar.Real(it) }
                else -> null
            }
            ChannelType.Int -> when (raw) {
                is Number -> OhdScalar.Int(raw.toLong())
                is Boolean -> OhdScalar.Int(if (raw) 1L else 0L)
                is String -> raw.toLongOrNull()?.let { OhdScalar.Int(it) }
                    ?: raw.toDoubleOrNull()?.let { OhdScalar.Int(it.toLong()) }
                else -> null
            }
            ChannelType.Bool -> when (raw) {
                is Boolean -> OhdScalar.Bool(raw)
                is Number -> OhdScalar.Bool(raw.toDouble() != 0.0)
                is String -> when (raw.lowercase()) {
                    "true", "yes", "1" -> OhdScalar.Bool(true)
                    "false", "no", "0" -> OhdScalar.Bool(false)
                    else -> null
                }
                else -> null
            }
            ChannelType.Text -> OhdScalar.Text(raw.toString())
        }
    }

    /**
     * Best-effort timestamp parser. Tries ISO-8601 first (covers RFC 3339
     * with offsets), then `yyyy-MM-dd HH:mm:ss`, then `yyyy-MM-dd`, then
     * raw Unix milliseconds. Returns `now` if everything fails — callers
     * shouldn't reach here unless `parseTimestamp` threw.
     */
    private fun parseTimestamp(raw: Any?): Long {
        if (raw == null) throw IllegalArgumentException("timestamp value is null")
        return when (raw) {
            is Number -> raw.toLong()
            is String -> parseTimestampString(raw)
            else -> parseTimestampString(raw.toString())
        }
    }

    private fun parseTimestampString(s: String): Long {
        val trimmed = s.trim()
        runCatching { return Instant.parse(trimmed).toEpochMilli() }
        runCatching {
            return LocalDateTime
                .parse(trimmed, DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss"))
                .toInstant(ZoneOffset.UTC)
                .toEpochMilli()
        }
        runCatching {
            return LocalDate
                .parse(trimmed, DateTimeFormatter.ofPattern("yyyy-MM-dd"))
                .atStartOfDay(ZoneOffset.UTC)
                .toInstant()
                .toEpochMilli()
        }
        trimmed.toLongOrNull()?.let { return it }
        throw IllegalArgumentException("unrecognised timestamp: $s")
    }
}
