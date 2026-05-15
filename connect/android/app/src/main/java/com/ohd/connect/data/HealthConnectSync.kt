package com.ohd.connect.data

import android.content.Context
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
import androidx.health.connect.client.request.ReadRecordsRequest
import androidx.health.connect.client.time.TimeRangeFilter
import java.time.Duration
import java.time.Instant
import kotlin.reflect.KClass

/**
 * Result of a [syncFromHealthConnect] run.
 *
 * - [readByType] — number of Health Connect records observed per record
 *   type, keyed by the OHD event type (e.g. `"activity.steps"`). One per
 *   type, even if zero, so the Settings screen can render a stable list.
 * - [ingested]   — total events handed to [StorageRepository.putEvent].
 *   May be less than the sum of [readByType] values if individual events
 *   failed (count goes to [errors]).
 * - [errors]     — human-readable error strings, one per failed type or
 *   ingest. Surfaced in the Debug section of the Health Connect
 *   settings screen.
 */
data class SyncResult(
    val readByType: Map<String, Int>,
    val ingested: Int,
    val errors: List<String>,
)

/**
 * Persisted sync metadata, stored in the same EncryptedSharedPreferences
 * file as `Auth`. `null` last-sync means "never synced".
 */
private const val KEY_LAST_SYNC_MS = "health_connect_last_sync_ms"
private const val KEY_TYPE_LAST_SYNC_PREFIX = "health_connect_last_sync_ms__"
// First-ever sync pulls a full 5-year history. Health Connect itself
// stores per-record retention up to ~30 days for some types and longer
// for others; the platform clamps the range, we just ask for "all".
// After this initial pull, subsequent runs use `lastSyncMs` so the
// window stays delta-only.
private const val DEFAULT_BACKFILL_DAYS = 365L * 5L

/**
 * Read records from Health Connect since the last sync (or 30 days if
 * never), turn each into an [EventInput], and persist via
 * [StorageRepository.putEvent].
 *
 * Per-type failures are isolated with [runCatching] so a single broken
 * record type can't abort the whole import. The function commits the new
 * `lastSyncMs` only after all eight reads complete; partial-progress
 * mid-failure isn't worth the complexity for v1.
 *
 * @param ctx     Application context — used for prefs + the HC client.
 * @param sinceMs Override the persisted last-sync timestamp; `null` reads
 *                from prefs (or defaults to 30 days ago).
 * @param untilMs Upper bound. Defaults to "now"; tests can pin it.
 */
suspend fun syncFromHealthConnect(
    ctx: Context,
    sinceMs: Long? = null,
    untilMs: Long = System.currentTimeMillis(),
): SyncResult {
    val client = OhdHealthConnect.client(ctx) ?: return SyncResult(
        readByType = emptyMap(),
        ingested = 0,
        errors = listOf("Health Connect provider not installed."),
    )

    val backfillSince = untilMs - Duration.ofDays(DEFAULT_BACKFILL_DAYS).toMillis()

    val readByType = mutableMapOf<String, Int>()
    val errors = mutableListOf<String>()
    var ingested = 0

    // ---- Helper that runs one record-type sync ----
    //
    // Each record type tracks its own `lastSyncMs` cursor in
    // [HealthConnectPrefs]. First call for a type defaults to the 5-year
    // backfill window — so granting a new permission later still pulls
    // history for that type (the original "granted Sleep after months,
    // got nothing" bug). The caller-supplied [sinceMs] overrides every
    // per-type cursor; used by the Settings "Sync now from scratch" button.
    suspend fun <T : Record> readType(
        recordKlass: KClass<T>,
        eventType: String,
        toEvents: (T) -> List<EventInput>,
    ) {
        // Ask storage for the latest event we already have of this type
        // from Health Connect. That timestamp is the incremental cursor —
        // no separate prefs counter to keep in sync. Self-healing across
        // reinstalls and any sync that committed events but failed before
        // updating a cursor: re-running picks up exactly where we left off.
        val latestStored = StorageRepository
            .queryEvents(
                EventFilter(
                    eventTypesIn = listOf(eventType),
                    sourceIn = listOf(SOURCE_TAG),
                    limit = 1,
                    visibility = EventVisibility.All,
                ),
            )
            .getOrNull()
            ?.firstOrNull()
            ?.timestampMs
        val effectiveSince = sinceMs
            ?: latestStored?.plus(1) // +1 ms so we don't re-pull the same record
            ?: backfillSince
        val timeRange = TimeRangeFilter.between(
            Instant.ofEpochMilli(effectiveSince),
            Instant.ofEpochMilli(untilMs),
        )
        // Page through results. Health Connect's `ReadRecordsRequest` caps
        // at 5000 rows per call and exposes a `pageToken` for the next
        // batch. The default 1000 was easy to overflow on a 5-year
        // backfill of HR samples — the watch records one HR record per
        // workout / minute, and each can carry 100+ sub-samples.
        val outcome = runCatching {
            var totalRecords = 0
            var pageToken: String? = null
            while (true) {
                val request = ReadRecordsRequest(
                    recordType = recordKlass,
                    timeRangeFilter = timeRange,
                    pageSize = 5_000,
                    pageToken = pageToken,
                )
                val response = client.readRecords(request)
                totalRecords += response.records.size
                for (record in response.records) {
                    val events = toEvents(record)
                    for (input in events) {
                        val res = StorageRepository.putEvent(input)
                        if (res.isSuccess) {
                            when (val o = res.getOrNull()) {
                                is PutEventOutcome.Committed,
                                is PutEventOutcome.Pending -> ingested++
                                is PutEventOutcome.Error -> errors.add(
                                    "$eventType: storage error ${o.code}: ${o.message}",
                                )
                                null -> Unit
                            }
                        } else {
                            errors.add(
                                "$eventType: putEvent threw ${res.exceptionOrNull()?.message ?: "(null)"}",
                            )
                        }
                    }
                }
                pageToken = response.pageToken
                if (pageToken == null) break
            }
            readByType[eventType] = totalRecords
        }
        // No per-type cursor to maintain — the watermark is the latest
        // stored event itself (see `latestStored` above).
        if (outcome.isFailure) {
            readByType.putIfAbsent(eventType, 0)
            errors.add(
                "$eventType: read failed — ${outcome.exceptionOrNull()?.message ?: "(null)"}",
            )
        }
    }

    // ---- Steps ----
    readType(StepsRecord::class, EVT_STEPS) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
                    .coerceAtLeast(0L),
                eventType = EVT_STEPS,
                channels = listOf(
                    EventChannelInput(
                        path = "count",
                        scalar = OhdScalar.Int(rec.count),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Heart rate ----
    //
    // A HeartRateRecord carries 1+ samples covering [startTime, endTime].
    // Fan out one OHD event per sample so the per-bpm timestamp survives.
    readType(HeartRateRecord::class, EVT_HEART_RATE) { rec ->
        rec.samples.map { sample ->
            EventInput(
                timestampMs = sample.time.toEpochMilli(),
                eventType = EVT_HEART_RATE,
                channels = listOf(
                    EventChannelInput(
                        path = "bpm",
                        scalar = OhdScalar.Real(sample.beatsPerMinute.toDouble()),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${sample.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    }

    // ---- Blood pressure ----
    readType(BloodPressureRecord::class, EVT_BLOOD_PRESSURE) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BLOOD_PRESSURE,
                channels = listOf(
                    EventChannelInput(
                        path = "systolic_mmhg",
                        scalar = OhdScalar.Real(rec.systolic.inMillimetersOfMercury),
                    ),
                    EventChannelInput(
                        path = "diastolic_mmhg",
                        scalar = OhdScalar.Real(rec.diastolic.inMillimetersOfMercury),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Blood glucose ----
    //
    // OHD canonicalises glucose to mmol/L. Health Connect's BloodGlucose unit
    // ships an mmol-per-litre accessor as a `Double`.
    readType(BloodGlucoseRecord::class, EVT_GLUCOSE) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_GLUCOSE,
                channels = listOf(
                    EventChannelInput(
                        path = "value",
                        scalar = OhdScalar.Real(rec.level.inMillimolesPerLiter),
                    ),
                    EventChannelInput(
                        path = "unit",
                        scalar = OhdScalar.Text("mmol/L"),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Weight ----
    readType(WeightRecord::class, EVT_WEIGHT) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_WEIGHT,
                channels = listOf(
                    EventChannelInput(
                        path = "kg",
                        scalar = OhdScalar.Real(rec.weight.inKilograms),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Body temperature ----
    readType(BodyTemperatureRecord::class, EVT_TEMPERATURE) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_TEMPERATURE,
                channels = listOf(
                    EventChannelInput(
                        path = "celsius",
                        scalar = OhdScalar.Real(rec.temperature.inCelsius),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Sleep ----
    //
    // A SleepSessionRecord has its own start/end. We emit a single
    // `activity.sleep` event with `duration_minutes` rather than fanning
    // out per-stage; clinician views can drill into Health Connect for
    // detail when they care.
    readType(SleepSessionRecord::class, EVT_SLEEP) { rec ->
        val durationMs = Duration.between(rec.startTime, rec.endTime).toMillis()
            .coerceAtLeast(0L)
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = durationMs,
                eventType = EVT_SLEEP,
                channels = listOf(
                    EventChannelInput(
                        path = "duration_minutes",
                        scalar = OhdScalar.Int(durationMs / 60_000L),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata, fallbackTitle = rec.title),
            ),
        )
    }

    // ---- Oxygen saturation ----
    readType(OxygenSaturationRecord::class, EVT_SPO2) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_SPO2,
                channels = listOf(
                    EventChannelInput(
                        path = "percentage",
                        scalar = OhdScalar.Real(rec.percentage.value),
                    ),
                ),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Resting heart rate ----
    readType(RestingHeartRateRecord::class, EVT_RESTING_HEART_RATE) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_RESTING_HEART_RATE,
                channels = listOf(EventChannelInput("bpm", OhdScalar.Real(rec.beatsPerMinute.toDouble()))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Heart rate variability (RMSSD) ----
    readType(HeartRateVariabilityRmssdRecord::class, EVT_HRV_RMSSD) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_HRV_RMSSD,
                channels = listOf(EventChannelInput("rmssd_ms", OhdScalar.Real(rec.heartRateVariabilityMillis))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Respiratory rate ----
    readType(RespiratoryRateRecord::class, EVT_RESPIRATORY_RATE) { rec ->
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
    }

    // ---- Basal body temperature ----
    readType(BasalBodyTemperatureRecord::class, EVT_BASAL_BODY_TEMP) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BASAL_BODY_TEMP,
                channels = listOf(EventChannelInput("celsius", OhdScalar.Real(rec.temperature.inCelsius))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Body composition ----
    readType(HeightRecord::class, EVT_HEIGHT) { rec ->
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
    }
    readType(BodyFatRecord::class, EVT_BODY_FAT) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BODY_FAT,
                channels = listOf(EventChannelInput("percentage", OhdScalar.Real(rec.percentage.value))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(BodyWaterMassRecord::class, EVT_BODY_WATER_MASS) { rec ->
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
    }
    readType(BoneMassRecord::class, EVT_BONE_MASS) { rec ->
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
    }
    readType(LeanBodyMassRecord::class, EVT_LEAN_BODY_MASS) { rec ->
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
    }

    // ---- Activity ----
    readType(DistanceRecord::class, EVT_DISTANCE) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_DISTANCE,
                channels = listOf(EventChannelInput("meters", OhdScalar.Real(rec.distance.inMeters))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(ElevationGainedRecord::class, EVT_ELEVATION) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_ELEVATION,
                channels = listOf(EventChannelInput("meters", OhdScalar.Real(rec.elevation.inMeters))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(FloorsClimbedRecord::class, EVT_FLOORS) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_FLOORS,
                channels = listOf(EventChannelInput("count", OhdScalar.Real(rec.floors))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(ActiveCaloriesBurnedRecord::class, EVT_ACTIVE_CALORIES) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_ACTIVE_CALORIES,
                channels = listOf(EventChannelInput("kcal", OhdScalar.Real(rec.energy.inKilocalories))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(TotalCaloriesBurnedRecord::class, EVT_TOTAL_CALORIES) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_TOTAL_CALORIES,
                channels = listOf(EventChannelInput("kcal", OhdScalar.Real(rec.energy.inKilocalories))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(BasalMetabolicRateRecord::class, EVT_BMR) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_BMR,
                channels = listOf(EventChannelInput("kcal_per_day", OhdScalar.Real(rec.basalMetabolicRate.inKilocaloriesPerDay))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(Vo2MaxRecord::class, EVT_VO2_MAX) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.time.toEpochMilli(),
                eventType = EVT_VO2_MAX,
                channels = listOf(EventChannelInput("ml_per_kg_per_min", OhdScalar.Real(rec.vo2MillilitersPerMinuteKilogram))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(ExerciseSessionRecord::class, EVT_EXERCISE) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
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
    }
    readType(PowerRecord::class, EVT_POWER) { rec ->
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
    }
    readType(SpeedRecord::class, EVT_SPEED) { rec ->
        rec.samples.map { s ->
            EventInput(
                timestampMs = s.time.toEpochMilli(),
                eventType = EVT_SPEED,
                channels = listOf(EventChannelInput("m_per_s", OhdScalar.Real(s.speed.inMetersPerSecond))),
                source = SOURCE_TAG,
                sourceId = "${rec.metadata.id}:${s.time.toEpochMilli()}",
                notes = sourceNote(rec.metadata),
            )
        }
    }
    readType(StepsCadenceRecord::class, EVT_STEPS_CADENCE) { rec ->
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
    }
    readType(CyclingPedalingCadenceRecord::class, EVT_CYCLING_CADENCE) { rec ->
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
    }
    readType(WheelchairPushesRecord::class, EVT_WHEELCHAIR_PUSHES) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_WHEELCHAIR_PUSHES,
                channels = listOf(EventChannelInput("count", OhdScalar.Int(rec.count))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // ---- Nutrition / hydration ----
    readType(NutritionRecord::class, EVT_NUTRITION) { rec ->
        val chans = mutableListOf<EventChannelInput>()
        rec.energy?.inKilocalories?.let { chans += EventChannelInput("kcal", OhdScalar.Real(it)) }
        rec.totalCarbohydrate?.inGrams?.let { chans += EventChannelInput("carbs_g", OhdScalar.Real(it)) }
        rec.protein?.inGrams?.let { chans += EventChannelInput("protein_g", OhdScalar.Real(it)) }
        rec.totalFat?.inGrams?.let { chans += EventChannelInput("fat_g", OhdScalar.Real(it)) }
        rec.sugar?.inGrams?.let { chans += EventChannelInput("sugar_g", OhdScalar.Real(it)) }
        rec.dietaryFiber?.inGrams?.let { chans += EventChannelInput("fiber_g", OhdScalar.Real(it)) }
        rec.caffeine?.inGrams?.let { chans += EventChannelInput("caffeine_mg", OhdScalar.Real(it * 1000.0)) }
        rec.name?.takeIf { it.isNotBlank() }?.let { chans += EventChannelInput("name", OhdScalar.Text(it)) }
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_NUTRITION,
                channels = chans,
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }
    readType(HydrationRecord::class, EVT_HYDRATION) { rec ->
        listOf(
            EventInput(
                timestampMs = rec.startTime.toEpochMilli(),
                durationMs = Duration.between(rec.startTime, rec.endTime).toMillis().coerceAtLeast(0L),
                eventType = EVT_HYDRATION,
                channels = listOf(EventChannelInput("liters", OhdScalar.Real(rec.volume.inLiters))),
                source = SOURCE_TAG,
                sourceId = rec.metadata.id,
                notes = sourceNote(rec.metadata),
            ),
        )
    }

    // Persist the upper bound so the next sync only reads new data.
    HealthConnectPrefs.setLastSyncMs(ctx, untilMs)

    return SyncResult(
        readByType = readByType,
        ingested = ingested,
        errors = errors,
    )
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
// Pref-backed sync metadata. Kept out of `Auth` so the Health Connect file
// stays self-contained, but we reuse `Auth.prefs(...)` access via a thin
// SharedPreferences read — they share the same EncryptedSharedPreferences
// file by name.
// =============================================================================

internal object HealthConnectPrefs {

    /**
     * Per-type cursor used by [readType]. Each record type tracks its own
     * last-sync timestamp so adding a new permission type later doesn't
     * skip the 5-year backfill the way a global cursor did (the original
     * bug: granting Sleep after months of Steps-only sync would only read
     * sleep from "now" onwards).
     */
    fun typeLastSyncMs(ctx: Context, eventType: String): Long? {
        val raw = prefs(ctx).getLong("$KEY_TYPE_LAST_SYNC_PREFIX$eventType", 0L)
        return raw.takeIf { it > 0 }
    }

    fun setTypeLastSyncMs(ctx: Context, eventType: String, ms: Long) {
        prefs(ctx).edit().putLong("$KEY_TYPE_LAST_SYNC_PREFIX$eventType", ms).apply()
    }

    /**
     * Latest of all per-type cursors — what the Settings screen displays
     * as "last sync". Falls back to the legacy global cursor for installs
     * that haven't yet sync'd under the new per-type scheme.
     */
    fun lastSyncMs(ctx: Context): Long? {
        val all = prefs(ctx).all
        val latest = all.entries
            .filter { it.key.startsWith(KEY_TYPE_LAST_SYNC_PREFIX) }
            .mapNotNull { (it.value as? Long)?.takeIf { v -> v > 0 } }
            .maxOrNull()
        if (latest != null) return latest
        val legacy = prefs(ctx).getLong(KEY_LAST_SYNC_MS, 0L)
        return legacy.takeIf { it > 0 }
    }

    /** Update the legacy global cursor too — Settings reads it for display. */
    fun setLastSyncMs(ctx: Context, ms: Long) {
        prefs(ctx).edit().putLong(KEY_LAST_SYNC_MS, ms).apply()
    }

    /** Wipe every per-type cursor — forces a full backfill on the next run. */
    fun clearAllTypeCursors(ctx: Context) {
        val editor = prefs(ctx).edit()
        prefs(ctx).all.keys
            .filter { it.startsWith(KEY_TYPE_LAST_SYNC_PREFIX) }
            .forEach { editor.remove(it) }
        editor.remove(KEY_LAST_SYNC_MS)
        editor.apply()
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(
        "ohd_health_connect",
        Context.MODE_PRIVATE,
    )
}
