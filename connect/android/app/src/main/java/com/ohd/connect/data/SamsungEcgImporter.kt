package com.ohd.connect.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runInterruptible
import java.io.BufferedReader
import java.io.InputStream
import java.io.InputStreamReader
import java.nio.charset.StandardCharsets
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** Outcome of [SamsungEcgImporter.importStrip]. */
data class ImportResult(
    val secondsEmitted: Int,
    val skippedDuplicate: Boolean,
    val error: String?,
)

/**
 * Parser + emitter for Samsung Health Monitor ECG CSV strips.
 *
 * Reads the UTF-8 BOM-prefixed `key,value` header + one-float-per-line sample
 * body, then emits one `measurement.ecg_second` event per second of waveform
 * via [StorageRepository.putEvent]. Reimports are idempotent: the
 * `correlation_id` channel is deterministic from `Created time`, so a second
 * pass over the same file is detected and skipped.
 */
object SamsungEcgImporter {

    private const val EVENT_TYPE = "measurement.ecg_second"
    private const val SOURCE_KIND = "samsung_health_monitor"

    /** Parsed header + raw mV samples for one strip. */
    data class StripMetadata(
        val createdTimeMs: Long,
        val avgHeartRate: Double?,
        val classification: String?,
        val symptoms: String?,
        val softwareVersion: String?,
        val device: String?,
        val samplingRateHz: Double,
        val lead: String,
        val unit: String,
        val sampleCount: Int,
        val durationSec: Double,
        val correlationId: String,
    )

    private val HEADER_TS_FORMATTER: DateTimeFormatter =
        DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss", Locale.US)

    /** Parse one Samsung Health Monitor ECG CSV into metadata + raw samples. */
    fun parse(stream: InputStream): Result<Pair<StripMetadata, FloatArray>> = runCatching {
        val reader = BufferedReader(InputStreamReader(stream, StandardCharsets.UTF_8))
        val lines = reader.readLines().map { it.trimBom() }

        val headerMap = mutableMapOf<String, String>()
        var i = 0
        // First header block: key,value rows until the first blank line.
        while (i < lines.size && lines[i].isNotBlank()) {
            val (k, v) = splitKv(lines[i])
            headerMap[k] = v
            i++
        }
        // Skip blanks.
        while (i < lines.size && lines[i].isBlank()) i++
        // Second header block: Lead,...  Unit,...
        while (i < lines.size && lines[i].isNotBlank()) {
            val (k, v) = splitKv(lines[i])
            headerMap[k] = v
            i++
        }
        // Skip blanks before the sample body.
        while (i < lines.size && lines[i].isBlank()) i++

        val createdTimeRaw = headerMap["Created time"]
            ?: error("missing 'Created time' header")
        val createdTimeMs = LocalDateTime
            .parse(createdTimeRaw, HEADER_TS_FORMATTER)
            .atZone(ZoneId.systemDefault())
            .toInstant()
            .toEpochMilli()

        val samplingRateHz = headerMap["Sampling rate"]
            ?.let { parseSamplingRate(it) }
            ?: error("missing 'Sampling rate' header")
        val lead = headerMap["Lead"] ?: "Lead I"
        val unit = headerMap["Unit"] ?: "mV"

        val samples = FloatArray(lines.size - i)
        var n = 0
        while (i < lines.size) {
            val ln = lines[i++].trim().trimEnd(',')
            if (ln.isBlank()) continue
            val f = ln.toFloatOrNull()
            // Samsung Health Monitor appends a `,` sentinel as the last
            // line after the 15 000 samples. Any non-numeric tail counts
            // as end-of-data — bail instead of failing the import.
                ?: break
            samples[n++] = f
        }
        val trimmed = samples.copyOf(n)

        val meta = StripMetadata(
            createdTimeMs = createdTimeMs,
            avgHeartRate = headerMap["Average heart rate"]?.toDoubleOrNull(),
            classification = headerMap["Classification"]?.takeIf { it.isNotBlank() },
            symptoms = headerMap["Symptoms"]?.takeIf { it.isNotBlank() },
            softwareVersion = headerMap["Software version"]?.takeIf { it.isNotBlank() },
            device = headerMap["Device"]?.takeIf { it.isNotBlank() },
            samplingRateHz = samplingRateHz,
            lead = lead,
            unit = unit,
            sampleCount = n,
            durationSec = n / samplingRateHz,
            correlationId = "SAMSUNG_ECG-$createdTimeMs",
        )
        meta to trimmed
    }

    /**
     * Emit one `measurement.ecg_second` event per second of waveform.
     *
     * Returns counts + a duplicate flag. The dedupe query looks for any
     * existing event with the same `correlation_id` channel within a 24 h
     * window around the strip's `createdTimeMs`; if anything matches we
     * skip and return `skippedDuplicate = true`.
     */
    suspend fun importStrip(meta: StripMetadata, samples: FloatArray): ImportResult {
        val existing = duplicateExists(meta)
        if (existing) return ImportResult(secondsEmitted = 0, skippedDuplicate = true, error = null)

        val rate = meta.samplingRateHz.toInt().coerceAtLeast(1)
        val minTail = rate / 2
        var emitted = 0
        var idx = 0
        var secondIndex = 0
        while (idx < samples.size) {
            val end = (idx + rate).coerceAtMost(samples.size)
            val count = end - idx
            if (count < minTail) break
            val chunk = FloatArray(count)
            System.arraycopy(samples, idx, chunk, 0, count)
            val outcome = runInterruptible(Dispatchers.IO) {
                StorageRepository.putEvent(buildEvent(meta, secondIndex, chunk))
            }
            outcome.fold(
                onSuccess = { res ->
                    when (res) {
                        is PutEventOutcome.Error -> return ImportResult(
                            secondsEmitted = emitted,
                            skippedDuplicate = false,
                            error = "put_event failed: ${res.code} ${res.message}",
                        )
                        else -> emitted++
                    }
                },
                onFailure = { e ->
                    return ImportResult(
                        secondsEmitted = emitted,
                        skippedDuplicate = false,
                        error = e.message ?: e::class.simpleName.orEmpty(),
                    )
                },
            )
            idx = end
            secondIndex++
        }
        return ImportResult(secondsEmitted = emitted, skippedDuplicate = false, error = null)
    }

    /**
     * Lookup for an existing strip with this `correlation_id`.
     *
     * `EventFilter` has no per-channel predicate yet, so we query the
     * `measurement.ecg_second` event type in a window around the strip's
     * `createdTimeMs` and check `correlation_id` in code. The window is
     * tight (one day each side) because Samsung strips are minutes long and
     * the correlation_id is timestamp-derived.
     */
    private fun duplicateExists(meta: StripMetadata): Boolean {
        val day = 86_400_000L
        val rows = StorageRepository.queryEvents(
            EventFilter(
                fromMs = meta.createdTimeMs - day,
                toMs = meta.createdTimeMs + day,
                eventTypesIn = listOf(EVENT_TYPE),
                limit = 100,
            ),
        ).getOrNull().orEmpty()
        return rows.any { ev ->
            ev.channels.any { ch ->
                ch.path == "correlation_id" &&
                    (ch.scalar as? OhdScalar.Text)?.v == meta.correlationId
            }
        }
    }

    private fun buildEvent(meta: StripMetadata, secondIndex: Int, chunk: FloatArray): EventInput {
        val timestampMs = meta.createdTimeMs + secondIndex * 1000L
        val samplesText = chunk.joinToString(",") { String.format(Locale.US, "%.6g", it) }
        val channels = buildList {
            add(EventChannelInput("correlation_id", OhdScalar.Text(meta.correlationId)))
            add(EventChannelInput("second_index", OhdScalar.Int(secondIndex.toLong())))
            add(EventChannelInput("samples_mv", OhdScalar.Text(samplesText)))
            add(EventChannelInput("sampling_rate_hz", OhdScalar.Real(meta.samplingRateHz)))
            add(EventChannelInput("lead", OhdScalar.Text(meta.lead)))
            meta.avgHeartRate?.let {
                add(EventChannelInput("avg_heart_rate", OhdScalar.Real(it)))
            }
            meta.classification?.let {
                add(EventChannelInput("classification", OhdScalar.Text(it)))
            }
            meta.symptoms?.let {
                add(EventChannelInput("symptoms", OhdScalar.Text(it)))
            }
            meta.device?.let {
                add(EventChannelInput("device", OhdScalar.Text(it)))
            }
            meta.softwareVersion?.let {
                add(EventChannelInput("software_version", OhdScalar.Text(it)))
            }
            add(EventChannelInput("source_kind", OhdScalar.Text(SOURCE_KIND)))
        }
        return EventInput(
            timestampMs = timestampMs,
            durationMs = 1000L,
            eventType = EVENT_TYPE,
            channels = channels,
            source = "import:$SOURCE_KIND",
            sourceId = meta.correlationId,
        )
    }

    private fun splitKv(line: String): Pair<String, String> {
        val idx = line.indexOf(',')
        return if (idx < 0) line to "" else line.substring(0, idx).trim() to line.substring(idx + 1).trim()
    }

    private fun parseSamplingRate(raw: String): Double {
        // "500.000 Hz" → 500.0. Strip everything after the first whitespace.
        val token = raw.trim().substringBefore(' ').trim()
        return token.toDouble()
    }

    private fun String.trimBom(): String =
        if (isNotEmpty() && this[0] == '﻿') substring(1) else this
}
