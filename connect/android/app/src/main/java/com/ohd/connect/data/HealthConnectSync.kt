package com.ohd.connect.data

import android.content.Context
import androidx.health.connect.client.HealthConnectClient
import androidx.health.connect.client.changes.DeletionChange
import androidx.health.connect.client.changes.UpsertionChange
import androidx.health.connect.client.records.ActiveCaloriesBurnedRecord
import androidx.health.connect.client.records.BasalBodyTemperatureRecord
import androidx.health.connect.client.records.BasalMetabolicRateRecord
import androidx.health.connect.client.records.BloodGlucoseRecord
import androidx.health.connect.client.records.BloodPressureRecord
import androidx.health.connect.client.records.BodyFatRecord
import androidx.health.connect.client.records.BodyTemperatureRecord
import androidx.health.connect.client.records.BodyWaterMassRecord
import androidx.health.connect.client.records.BoneMassRecord
import androidx.health.connect.client.records.CyclingPedalingCadenceRecord
import androidx.health.connect.client.records.DistanceRecord
import androidx.health.connect.client.records.ElevationGainedRecord
import androidx.health.connect.client.records.ExerciseSessionRecord
import androidx.health.connect.client.records.FloorsClimbedRecord
import androidx.health.connect.client.records.HeartRateRecord
import androidx.health.connect.client.records.HeartRateVariabilityRmssdRecord
import androidx.health.connect.client.records.HeightRecord
import androidx.health.connect.client.records.HydrationRecord
import androidx.health.connect.client.records.LeanBodyMassRecord
import androidx.health.connect.client.records.NutritionRecord
import androidx.health.connect.client.records.OxygenSaturationRecord
import androidx.health.connect.client.records.PowerRecord
import androidx.health.connect.client.records.Record
import androidx.health.connect.client.records.RespiratoryRateRecord
import androidx.health.connect.client.records.RestingHeartRateRecord
import androidx.health.connect.client.records.SleepSessionRecord
import androidx.health.connect.client.records.SpeedRecord
import androidx.health.connect.client.records.StepsCadenceRecord
import androidx.health.connect.client.records.StepsRecord
import androidx.health.connect.client.records.TotalCaloriesBurnedRecord
import androidx.health.connect.client.records.Vo2MaxRecord
import androidx.health.connect.client.records.WeightRecord
import androidx.health.connect.client.records.WheelchairPushesRecord
import androidx.health.connect.client.records.metadata.Metadata
import androidx.health.connect.client.request.ChangesTokenRequest
import androidx.health.connect.client.request.ReadRecordsRequest
import androidx.health.connect.client.time.TimeRangeFilter
import java.time.Duration
import java.time.Instant
import kotlin.reflect.KClass

/**
 * Result of a [syncFromHealthConnect] run.
 *
 * - [readByType] — number of OHD events derived per record type for this
 *   run, keyed by the OHD event type (e.g. `"activity.steps"`). One entry
 *   per known type, even if zero, so the Settings screen can render a
 *   stable list.
 * - [ingested]   — total events handed to [StorageRepository.putEvent].
 *   May be less than the sum of [readByType] values if individual events
 *   failed (count goes to [errors]).
 * - [errors]     — human-readable error strings, one per failed type or
 *   ingest. Surfaced in the Debug section of the Health Connect
 *   settings screen.
 * - [mode]       — how this run discovered records: a one-time historical
 *   backfill, or an incremental Changes-API delta.
 * - [changesProcessed] — number of Health Connect `Change` rows consumed
 *   from the Changes API (`UpsertionChange` + `DeletionChange`). Zero for
 *   a pure historical backfill.
 * - [deletions]  — number of `DeletionChange` rows observed (records
 *   removed in Health Connect since the last sync).
 * - [tokenAcquired] — whether this run ended holding a valid changes
 *   token; surfaced in the Debug card so the user can see incremental
 *   sync is armed.
 */
data class SyncResult(
    val readByType: Map<String, Int>,
    val ingested: Int,
    val errors: List<String>,
    val mode: SyncMode = SyncMode.Incremental,
    val changesProcessed: Int = 0,
    val deletions: Int = 0,
    val tokenAcquired: Boolean = false,
)

/** How a [syncFromHealthConnect] run discovered records to ingest. */
enum class SyncMode {
    /** One-time bounded historical read — first sync, or after token expiry. */
    HistoricalBackfill,

    /** Delta read via the Health Connect Changes API. */
    Incremental,
}

/**
 * Persisted sync metadata, stored in the same `SharedPreferences` file as
 * the rest of the Health Connect state.
 */
private const val KEY_LAST_SYNC_MS = "health_connect_last_sync_ms"
private const val KEY_CHANGES_TOKEN = "health_connect_changes_token"
// The first-ever sync pulls a bounded history. Health Connect itself
// stores per-record retention up to ~30 days for some types and longer
// for others; the platform clamps the range, we just ask for "all".
// After this initial pull, every subsequent run is delta-only via the
// Changes API — see the function doc below.
private const val DEFAULT_BACKFILL_DAYS = 365L * 5L

/**
 * Pull fresh data from Health Connect and persist it as OHD events.
 *
 * ## Why the Changes API
 *
 * The old approach used a per-record-type timestamp watermark: read every
 * record with `timestamp > latest stored event`. That silently dropped
 * data. Samsung Health drip-feeds samples into Health Connect *after* a
 * sync has run, stamped with times that fall *below* the current
 * watermark — heart-rate especially — so those samples were skipped
 * forever.
 *
 * Health Connect's Changes API hands changes back in **insertion order**,
 * independent of record timestamp, so a late-arriving backdated sample is
 * still delivered. This function uses it for every sync after the first.
 *
 * ## Flow
 *
 *  - **Initial sync** (no stored changes token): do a one-time bounded
 *    historical read ([DEFAULT_BACKFILL_DAYS]) so existing history lands,
 *    then acquire a changes token. Every later run is incremental.
 *  - **Incremental sync** (token present): loop `getChanges` while
 *    `hasMore`, advancing the token, and persist `nextChangesToken`.
 *  - **Token expired** (`changesTokenExpired == true`, or `getChanges`
 *    throws for a stale token): discard it, do a fresh historical read,
 *    re-acquire a token.
 *
 * Per-type failures are isolated with [runCatching] so one broken record
 * type can't abort the whole import.
 *
 * @param ctx     Application context — used for prefs + the HC client.
 * @param forceHistorical Force the historical-backfill path even when a
 *                changes token exists; backs the Settings "Sync from
 *                scratch" affordance. Discards the stored token.
 * @param untilMs Upper bound for the historical read. Defaults to "now";
 *                tests can pin it.
 */
suspend fun syncFromHealthConnect(
    ctx: Context,
    forceHistorical: Boolean = false,
    untilMs: Long = System.currentTimeMillis(),
): SyncResult {
    val client = OhdHealthConnect.client(ctx) ?: return SyncResult(
        readByType = emptyMap(),
        ingested = 0,
        errors = listOf("Health Connect provider not installed."),
    )

    val ingest = IngestAccumulator()
    val storedToken = HealthConnectPrefs.changesToken(ctx)
        ?.takeUnless { forceHistorical }

    if (storedToken == null) {
        // ---- Initial sync (or forced "from scratch") ----------------
        // No token yet: pull a bounded history once, then arm the
        // Changes API for every future run.
        if (forceHistorical) HealthConnectPrefs.clearChangesToken(ctx)
        historicalBackfill(client, ingest, untilMs)
        val token = acquireChangesToken(ctx, client, ingest)
        HealthConnectPrefs.setLastSyncMs(ctx, untilMs)
        return ingest.toResult(SyncMode.HistoricalBackfill, tokenAcquired = token != null)
    }

    // ---- Incremental sync via the Changes API -----------------------
    val expired = drainChanges(ctx, client, storedToken, ingest)
    if (expired) {
        // The provider rotated us out (default retention is ~30 days of
        // change history). Fall back to a fresh historical read and
        // re-arm — same recovery path as a never-synced install.
        ingest.errors.add(
            "Changes token expired — ran a fresh historical backfill and re-armed.",
        )
        HealthConnectPrefs.clearChangesToken(ctx)
        ingest.reset()
        historicalBackfill(client, ingest, untilMs)
        val token = acquireChangesToken(ctx, client, ingest)
        HealthConnectPrefs.setLastSyncMs(ctx, untilMs)
        return ingest.toResult(SyncMode.HistoricalBackfill, tokenAcquired = token != null)
    }

    HealthConnectPrefs.setLastSyncMs(ctx, untilMs)
    return ingest.toResult(
        SyncMode.Incremental,
        tokenAcquired = HealthConnectPrefs.changesToken(ctx) != null,
    )
}

/**
 * Acquire a fresh Changes-API token covering every record type we map and
 * persist it. Returns the token, or `null` on failure (error appended to
 * [ingest]). A `null` here just means the next run retries the historical
 * path — no data is lost.
 */
private suspend fun acquireChangesToken(
    ctx: Context,
    client: HealthConnectClient,
    ingest: IngestAccumulator,
): String? {
    return runCatching {
        val token = client.getChangesToken(
            ChangesTokenRequest(recordTypes = RECORD_MAPPERS.keys),
        )
        HealthConnectPrefs.setChangesToken(ctx, token)
        token
    }.getOrElse { e ->
        ingest.errors.add("Failed to acquire changes token — ${e.message ?: "(null)"}")
        null
    }
}

/**
 * Loop `getChanges` from [startToken] while `hasMore`, advancing and
 * persisting the token after each page so a crash mid-drain resumes
 * cleanly. Each [UpsertionChange] is mapped + ingested; each
 * [DeletionChange] is counted (storage has no per-source-id delete yet —
 * see [IngestAccumulator.deletions]).
 *
 * @return `true` if the provider reported the token expired — the caller
 *         should fall back to a historical backfill.
 */
private suspend fun drainChanges(
    ctx: Context,
    client: HealthConnectClient,
    startToken: String,
    ingest: IngestAccumulator,
): Boolean {
    var token = startToken
    while (true) {
        val response = try {
            client.getChanges(token)
        } catch (e: Exception) {
            // Some providers surface an expired token as a thrown
            // IllegalStateException rather than `changesTokenExpired`.
            // Treat any failure on the very first page as "expired" so we
            // recover instead of wedging; a mid-drain failure is logged
            // and retried by the worker on the next firing.
            return if (token == startToken) {
                true
            } else {
                ingest.errors.add("getChanges failed mid-drain — ${e.message ?: "(null)"}")
                false
            }
        }
        if (response.changesTokenExpired) return true

        for (change in response.changes) {
            when (change) {
                is UpsertionChange -> ingestRecord(change.record, ingest)
                is DeletionChange -> ingest.deletions++
                else -> Unit
            }
        }
        ingest.changesProcessed += response.changes.size

        token = response.nextChangesToken
        HealthConnectPrefs.setChangesToken(ctx, token)
        if (!response.hasMore) break
    }
    return false
}

/**
 * One-time bounded historical read across every mapped record type. Kept
 * from the original watermark implementation — it only runs on the first
 * sync (or after a token expiry), so paging the full window is fine.
 */
private suspend fun historicalBackfill(
    client: HealthConnectClient,
    ingest: IngestAccumulator,
    untilMs: Long,
) {
    val sinceMs = untilMs - Duration.ofDays(DEFAULT_BACKFILL_DAYS).toMillis()
    val timeRange = TimeRangeFilter.between(
        Instant.ofEpochMilli(sinceMs),
        Instant.ofEpochMilli(untilMs),
    )
    for ((recordKlass, _) in RECORD_MAPPERS) {
        readHistorical(client, recordKlass, timeRange, ingest)
    }
}

/**
 * Page through `readRecords` for one record type and ingest every row.
 * `ReadRecordsRequest` caps at 5000 rows per call; the watch can produce
 * far more HR samples than that across a 5-year window, so we follow the
 * `pageToken` until exhausted.
 */
private suspend fun <T : Record> readHistorical(
    client: HealthConnectClient,
    recordKlass: KClass<T>,
    timeRange: TimeRangeFilter,
    ingest: IngestAccumulator,
) {
    val outcome = runCatching {
        var pageToken: String? = null
        while (true) {
            val response = client.readRecords(
                ReadRecordsRequest(
                    recordType = recordKlass,
                    timeRangeFilter = timeRange,
                    pageSize = 5_000,
                    pageToken = pageToken,
                ),
            )
            for (record in response.records) {
                ingestRecord(record, ingest)
            }
            pageToken = response.pageToken
            if (pageToken == null) break
        }
    }
    if (outcome.isFailure) {
        ingest.errors.add(
            "${recordKlass.simpleName}: historical read failed — " +
                (outcome.exceptionOrNull()?.message ?: "(null)"),
        )
    }
}

/**
 * Map a single Health Connect [Record] to OHD [EventInput]s via
 * [RECORD_MAPPERS] and persist each through [StorageRepository.putEvent].
 *
 * `putEvent` is idempotent on `(source, sourceId)`, so a record delivered
 * by both the historical backfill and the first Changes-API page is
 * de-duplicated by storage — no extra bookkeeping here. Unknown record
 * types (mapper absent) are silently ignored.
 */
private fun ingestRecord(record: Record, ingest: IngestAccumulator) {
    val mapper = RECORD_MAPPERS[record::class] ?: return
    val events = runCatching { mapper(record) }.getOrElse { e ->
        ingest.errors.add("${record::class.simpleName}: mapping failed — ${e.message ?: "(null)"}")
        return
    }
    for (input in events) {
        ingest.readByType[input.eventType] =
            (ingest.readByType[input.eventType] ?: 0) + 1
        val res = StorageRepository.putEvent(input)
        if (res.isSuccess) {
            when (val o = res.getOrNull()) {
                is PutEventOutcome.Committed,
                is PutEventOutcome.Pending -> ingest.ingested++
                is PutEventOutcome.Error -> ingest.errors.add(
                    "${input.eventType}: storage error ${o.code}: ${o.message}",
                )
                null -> Unit
            }
        } else {
            ingest.errors.add(
                "${input.eventType}: putEvent threw " +
                    (res.exceptionOrNull()?.message ?: "(null)"),
            )
        }
    }
}

/** Mutable scratch state threaded through one [syncFromHealthConnect] run. */
private class IngestAccumulator {
    val readByType: MutableMap<String, Int> = mutableMapOf()
    val errors: MutableList<String> = mutableListOf()
    var ingested: Int = 0
    var changesProcessed: Int = 0
    var deletions: Int = 0

    /** Drop ingest counters before a post-expiry historical retry. */
    fun reset() {
        readByType.clear()
        ingested = 0
        changesProcessed = 0
        deletions = 0
        // `errors` is intentionally kept — the expiry note belongs in the result.
    }

    fun toResult(mode: SyncMode, tokenAcquired: Boolean): SyncResult {
        // Ensure every well-known type appears so the Settings list is stable.
        val byType = readByType.toMutableMap()
        for ((_, eventType) in HEALTH_CONNECT_TYPES) byType.putIfAbsent(eventType, 0)
        return SyncResult(
            readByType = byType,
            ingested = ingested,
            errors = errors,
            mode = mode,
            changesProcessed = changesProcessed,
            deletions = deletions,
            tokenAcquired = tokenAcquired,
        )
    }
}

/**
 * "source: Health Connect — {dataOriginPackageName}" note. Each Health
 * Connect record carries a `Metadata.dataOrigin.packageName` identifying
 * the originating wearable / app. We capture that so OHD events trace back
 * to the wearable that produced them.
 */
private fun sourceNote(metadata: Metadata, fallbackTitle: String? = null): String? {
    val pkg = metadata.dataOrigin.packageName.takeIf { it.isNotBlank() }
    val src = pkg ?: fallbackTitle ?: return "source: Health Connect"
    return "source: Health Connect — $src"
}

// =============================================================================
// Record-type → OHD-event mapping table.
//
// One entry per Health Connect record type we ingest. The same table drives
// both the historical backfill (`readRecords`) and the incremental Changes
// API (`getChanges`) — the *discovery* of which records to read differs, the
// mapping does not. Keys also seed `ChangesTokenRequest.recordTypes`.
// =============================================================================

/**
 * Cast helper so each mapper lambda can be written against its concrete
 * record type while the table stays `KClass<out Record> -> (Record) -> …`.
 */
@Suppress("UNCHECKED_CAST")
private fun <T : Record> mapper(fn: (T) -> List<EventInput>): (Record) -> List<EventInput> =
    fn as (Record) -> List<EventInput>

internal val RECORD_MAPPERS: Map<KClass<out Record>, (Record) -> List<EventInput>> = mapOf(
    // ---- Steps ----
    StepsRecord::class to mapper<StepsRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_STEPS,
                channels = listOf(EventChannelInput("count", OhdScalar.Int(rec.count))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Heart rate ----
    //
    // A HeartRateRecord carries 1+ samples covering [startTime, endTime].
    // Fan out one OHD event per sample so the per-bpm timestamp survives.
    HeartRateRecord::class to mapper<HeartRateRecord> { rec ->
        rec.samples.map { sample ->
            EventInput(
                timestampMs = sample.time.toEpochMilli(),
                eventType = EVT_HEART_RATE,
                channels = listOf(
                    EventChannelInput("bpm", OhdScalar.Real(sample.beatsPerMinute.toDouble())),
                ),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${sample.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    },

    // ---- Blood pressure ----
    BloodPressureRecord::class to mapper<BloodPressureRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BLOOD_PRESSURE,
                channels = listOf(
                    EventChannelInput(
                        "systolic_mmhg",
                        OhdScalar.Real(rec.systolic.inMillimetersOfMercury),
                    ),
                    EventChannelInput(
                        "diastolic_mmhg",
                        OhdScalar.Real(rec.diastolic.inMillimetersOfMercury),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Blood glucose ----
    //
    // OHD canonicalises glucose to mmol/L. Health Connect's BloodGlucose unit
    // ships an mmol-per-litre accessor as a `Double`.
    BloodGlucoseRecord::class to mapper<BloodGlucoseRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_GLUCOSE,
                channels = listOf(
                    EventChannelInput("value", OhdScalar.Real(rec.level.inMillimolesPerLiter)),
                    EventChannelInput("unit", OhdScalar.Text("mmol/L")),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Weight ----
    WeightRecord::class to mapper<WeightRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_WEIGHT,
                channels = listOf(EventChannelInput("kg", OhdScalar.Real(rec.weight.inKilograms))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Body temperature ----
    BodyTemperatureRecord::class to mapper<BodyTemperatureRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_TEMPERATURE,
                channels = listOf(
                    EventChannelInput("celsius", OhdScalar.Real(rec.temperature.inCelsius)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Sleep ----
    //
    // A SleepSessionRecord has its own start/end. We emit a single
    // `activity.sleep` event with `duration_minutes` rather than fanning
    // out per-stage; clinician views can drill into Health Connect for
    // detail when they care.
    SleepSessionRecord::class to mapper<SleepSessionRecord> { rec ->
        val durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
            .coerceAtLeast(0L)
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = durationMs,
                eventType = EVT_SLEEP,
                channels = listOf(
                    EventChannelInput("duration_minutes", OhdScalar.Int(durationMs / 60_000L)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata, fallbackTitle = rec.title),
            ),
        )
    },

    // ---- Oxygen saturation ----
    OxygenSaturationRecord::class to mapper<OxygenSaturationRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_SPO2,
                channels = listOf(
                    EventChannelInput("percentage", OhdScalar.Real(rec.percentage.value)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Resting heart rate ----
    RestingHeartRateRecord::class to mapper<RestingHeartRateRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_RESTING_HEART_RATE,
                channels = listOf(
                    EventChannelInput("bpm", OhdScalar.Real(rec.beatsPerMinute.toDouble())),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Heart rate variability (RMSSD) ----
    HeartRateVariabilityRmssdRecord::class to mapper<HeartRateVariabilityRmssdRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_HRV_RMSSD,
                channels = listOf(
                    EventChannelInput("rmssd_ms", OhdScalar.Real(rec.heartRateVariabilityMillis)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Respiratory rate ----
    RespiratoryRateRecord::class to mapper<RespiratoryRateRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_RESPIRATORY_RATE,
                channels = listOf(EventChannelInput("rate_per_min", OhdScalar.Real(rec.rate))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Basal body temperature ----
    BasalBodyTemperatureRecord::class to mapper<BasalBodyTemperatureRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BASAL_BODY_TEMP,
                channels = listOf(
                    EventChannelInput("celsius", OhdScalar.Real(rec.temperature.inCelsius)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Body composition ----
    HeightRecord::class to mapper<HeightRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_HEIGHT,
                channels = listOf(EventChannelInput("meters", OhdScalar.Real(rec.height.inMeters))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    BodyFatRecord::class to mapper<BodyFatRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BODY_FAT,
                channels = listOf(
                    EventChannelInput("percentage", OhdScalar.Real(rec.percentage.value)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    BodyWaterMassRecord::class to mapper<BodyWaterMassRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BODY_WATER_MASS,
                channels = listOf(EventChannelInput("kg", OhdScalar.Real(rec.mass.inKilograms))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    BoneMassRecord::class to mapper<BoneMassRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BONE_MASS,
                channels = listOf(EventChannelInput("kg", OhdScalar.Real(rec.mass.inKilograms))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    LeanBodyMassRecord::class to mapper<LeanBodyMassRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_LEAN_BODY_MASS,
                channels = listOf(EventChannelInput("kg", OhdScalar.Real(rec.mass.inKilograms))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Activity ----
    DistanceRecord::class to mapper<DistanceRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_DISTANCE,
                channels = listOf(
                    EventChannelInput("meters", OhdScalar.Real(rec.distance.inMeters)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    ElevationGainedRecord::class to mapper<ElevationGainedRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_ELEVATION,
                channels = listOf(
                    EventChannelInput("meters", OhdScalar.Real(rec.elevation.inMeters)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    FloorsClimbedRecord::class to mapper<FloorsClimbedRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_FLOORS,
                channels = listOf(EventChannelInput("count", OhdScalar.Real(rec.floors))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    ActiveCaloriesBurnedRecord::class to mapper<ActiveCaloriesBurnedRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_ACTIVE_CALORIES,
                channels = listOf(
                    EventChannelInput("kcal", OhdScalar.Real(rec.energy.inKilocalories)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    TotalCaloriesBurnedRecord::class to mapper<TotalCaloriesBurnedRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_TOTAL_CALORIES,
                channels = listOf(
                    EventChannelInput("kcal", OhdScalar.Real(rec.energy.inKilocalories)),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    BasalMetabolicRateRecord::class to mapper<BasalMetabolicRateRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BMR,
                channels = listOf(
                    EventChannelInput(
                        "kcal_per_day",
                        OhdScalar.Real(rec.basalMetabolicRate.inKilocaloriesPerDay),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    Vo2MaxRecord::class to mapper<Vo2MaxRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_VO2_MAX,
                channels = listOf(
                    EventChannelInput(
                        "ml_per_kg_per_min",
                        OhdScalar.Real(rec.vo2MillilitersPerMinuteKilogram),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    ExerciseSessionRecord::class to mapper<ExerciseSessionRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_EXERCISE,
                channels = listOf(
                    EventChannelInput("exercise_type", OhdScalar.Int(rec.exerciseType.toLong())),
                    EventChannelInput("title", OhdScalar.Text(rec.title ?: "")),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata, fallbackTitle = rec.title),
            ),
        )
    },
    PowerRecord::class to mapper<PowerRecord> { rec ->
        rec.samples.map { s ->
            EventInput(
                timestampMs = s.time.toEpochMilli(),
                eventType = EVT_POWER,
                channels = listOf(EventChannelInput("watts", OhdScalar.Real(s.power.inWatts))),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${s.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    },
    SpeedRecord::class to mapper<SpeedRecord> { rec ->
        rec.samples.map { s ->
            EventInput(
                timestampMs = s.time.toEpochMilli(),
                eventType = EVT_SPEED,
                channels = listOf(
                    EventChannelInput("m_per_s", OhdScalar.Real(s.speed.inMetersPerSecond)),
                ),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${s.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    },
    StepsCadenceRecord::class to mapper<StepsCadenceRecord> { rec ->
        rec.samples.map { s ->
            EventInput(
                timestampMs = s.time.toEpochMilli(),
                eventType = EVT_STEPS_CADENCE,
                channels = listOf(EventChannelInput("steps_per_min", OhdScalar.Real(s.rate))),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${s.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    },
    CyclingPedalingCadenceRecord::class to mapper<CyclingPedalingCadenceRecord> { rec ->
        rec.samples.map { s ->
            EventInput(
                timestampMs = s.time.toEpochMilli(),
                eventType = EVT_CYCLING_CADENCE,
                channels = listOf(EventChannelInput("rpm", OhdScalar.Real(s.revolutionsPerMinute))),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${s.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    },
    WheelchairPushesRecord::class to mapper<WheelchairPushesRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_WHEELCHAIR_PUSHES,
                channels = listOf(EventChannelInput("count", OhdScalar.Int(rec.count))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },

    // ---- Nutrition / hydration ----
    NutritionRecord::class to mapper<NutritionRecord> { rec ->
        val chans = mutableListOf<EventChannelInput>()
        rec.energy?.inKilocalories?.let { chans += EventChannelInput("kcal", OhdScalar.Real(it)) }
        rec.totalCarbohydrate?.inGrams?.let {
            chans += EventChannelInput("carbs_g", OhdScalar.Real(it))
        }
        rec.protein?.inGrams?.let { chans += EventChannelInput("protein_g", OhdScalar.Real(it)) }
        rec.totalFat?.inGrams?.let { chans += EventChannelInput("fat_g", OhdScalar.Real(it)) }
        rec.sugar?.inGrams?.let { chans += EventChannelInput("sugar_g", OhdScalar.Real(it)) }
        rec.dietaryFiber?.inGrams?.let { chans += EventChannelInput("fiber_g", OhdScalar.Real(it)) }
        rec.caffeine?.inGrams?.let {
            chans += EventChannelInput("caffeine_mg", OhdScalar.Real(it * 1000.0))
        }
        rec.name?.takeIf { it.isNotBlank() }?.let {
            chans += EventChannelInput("name", OhdScalar.Text(it))
        }
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_NUTRITION,
                channels = chans,
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
    HydrationRecord::class to mapper<HydrationRecord> { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_HYDRATION,
                channels = listOf(EventChannelInput("liters", OhdScalar.Real(rec.volume.inLiters))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    },
)

// =============================================================================
// Event-type constants. Kept here so the per-type mapping table sits next to
// the readers — the Settings screen also imports them for the per-type list.
// =============================================================================

const val EVT_STEPS = "activity.steps"
const val EVT_HEART_RATE = "measurement.heart_rate"
const val EVT_RESTING_HEART_RATE = "measurement.resting_heart_rate"
const val EVT_HRV_RMSSD = "measurement.hrv_rmssd"
const val EVT_BLOOD_PRESSURE = "measurement.blood_pressure"
const val EVT_GLUCOSE = "measurement.glucose"
const val EVT_WEIGHT = "measurement.weight"
const val EVT_TEMPERATURE = "measurement.temperature"
const val EVT_BASAL_BODY_TEMP = "measurement.basal_body_temperature"
const val EVT_SLEEP = "activity.sleep"
const val EVT_SPO2 = "measurement.spo2"
const val EVT_RESPIRATORY_RATE = "measurement.respiratory_rate"
const val EVT_HEIGHT = "measurement.height"
const val EVT_BODY_FAT = "measurement.body_fat"
const val EVT_BODY_WATER_MASS = "measurement.body_water_mass"
const val EVT_BONE_MASS = "measurement.bone_mass"
const val EVT_LEAN_BODY_MASS = "measurement.lean_body_mass"
const val EVT_DISTANCE = "activity.distance"
const val EVT_ELEVATION = "activity.elevation_gained"
const val EVT_FLOORS = "activity.floors_climbed"
const val EVT_EXERCISE = "activity.exercise_session"
const val EVT_ACTIVE_CALORIES = "activity.active_calories_burned"
const val EVT_TOTAL_CALORIES = "activity.total_calories_burned"
const val EVT_BMR = "measurement.basal_metabolic_rate"
const val EVT_VO2_MAX = "measurement.vo2_max"
const val EVT_POWER = "activity.power"
const val EVT_SPEED = "activity.speed"
const val EVT_STEPS_CADENCE = "activity.steps_cadence"
const val EVT_CYCLING_CADENCE = "activity.cycling_cadence"
const val EVT_WHEELCHAIR_PUSHES = "activity.wheelchair_pushes"
const val EVT_NUTRITION = "food.eaten"
const val EVT_HYDRATION = "activity.hydration"

/** Stable `EventInput.source` tag for all Health Connect ingest. */
const val SOURCE_TAG = "health_connect"

/** Display name + event-type pair, in the order they appear on the screen. */
val HEALTH_CONNECT_TYPES: List<Pair<String, String>> = listOf(
    "Steps" to EVT_STEPS,
    "Heart rate" to EVT_HEART_RATE,
    "Blood pressure" to EVT_BLOOD_PRESSURE,
    "Blood glucose" to EVT_GLUCOSE,
    "Weight" to EVT_WEIGHT,
    "Body temperature" to EVT_TEMPERATURE,
    "Sleep" to EVT_SLEEP,
    "Oxygen saturation" to EVT_SPO2,
)

// =============================================================================
// Pref-backed sync metadata.
//
// The Changes-API token is the only cursor we keep — it replaces the old
// per-record-type timestamp watermark, which silently dropped backdated
// samples drip-fed by Samsung Health. `lastSyncMs` is display-only.
// =============================================================================

internal object HealthConnectPrefs {

    /**
     * The persisted Health Connect Changes-API token. `null` means "never
     * synced" — [syncFromHealthConnect] then runs a one-time historical
     * backfill and acquires a fresh token. Every later run advances this
     * token as it drains `getChanges`.
     */
    fun changesToken(ctx: Context): String? =
        prefs(ctx).getString(KEY_CHANGES_TOKEN, null)?.takeIf { it.isNotEmpty() }

    fun setChangesToken(ctx: Context, token: String) {
        prefs(ctx).edit().putString(KEY_CHANGES_TOKEN, token).apply()
    }

    /** Drop the token — forces a historical backfill + re-arm on the next run. */
    fun clearChangesToken(ctx: Context) {
        prefs(ctx).edit().remove(KEY_CHANGES_TOKEN).apply()
    }

    /** Last-sync wall-clock timestamp — display only ("2 h ago" on Settings). */
    fun lastSyncMs(ctx: Context): Long? =
        prefs(ctx).getLong(KEY_LAST_SYNC_MS, 0L).takeIf { it > 0 }

    fun setLastSyncMs(ctx: Context, ms: Long) {
        prefs(ctx).edit().putLong(KEY_LAST_SYNC_MS, ms).apply()
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(
        "ohd_health_connect",
        Context.MODE_PRIVATE,
    )
}
