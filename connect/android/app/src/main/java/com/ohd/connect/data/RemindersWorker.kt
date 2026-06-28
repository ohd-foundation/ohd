package com.ohd.connect.data

import android.content.Context
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import org.json.JSONArray
import org.json.JSONObject
import java.util.Calendar
import java.util.concurrent.TimeUnit

/**
 * Periodic worker that checks the medication schedule and the daily-summary
 * window, then fires Android notifications (+ inbox entries) for whatever
 * the user has opted into.
 *
 * Scheduling is owned by [RemindersScheduler] — this class is just the
 * body of one tick. WorkManager invokes it every 30 minutes; we read the
 * three reminder toggles each time so the user's settings take effect
 * without having to cancel/reschedule the work.
 *
 * Three checks:
 *
 *  1. **Med reminders** (`reminders_meds_enabled_bool`, default true) —
 *     iterate the prescribed meds from [StubData.medications], find the
 *     last `medication.taken` event for each, and if `now > lastTaken +
 *     scheduleHours + 30 min` (i.e. the user is more than a half-hour
 *     late) and we haven't already fired this hour's reminder, append a
 *     `med_reminder` entry. Dedup keys live in [NotificationCenter.PREF_KEY_DEDUP]
 *     so the same dose doesn't fire on every tick.
 *
 *  2. **Daily summary** (`reminders_daily_summary_enabled_bool`, default
 *     false) — at or after 21:00 local, post one "{n} events logged today"
 *     entry. Dedup-keyed by date so we only fire once per calendar day.
 *
 *  3. **Calendar export** (`reminders_calendar_export_enabled_bool`,
 *     default false) — out of scope for v1; the worker logs a TODO and
 *     moves on.
 *
 * Failure handling mirrors [HealthConnectSyncWorker]: a missing storage
 * handle bails to [Result.retry] so the next firing picks the work up
 * once the user finishes onboarding; any other exception is logged and
 * also retries (WorkManager applies exponential backoff between retries).
 */
class RemindersWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {

    override suspend fun doWork(): Result = runCatching {
        val ctx = applicationContext

        // Reminders read events from the on-device storage core. In remote
        // storage mode the local core is absent — skip cleanly. (The reminder
        // engine itself is an on-device-only feature for Phase 3.)
        StorageRepository.init(ctx)
        if (StorageRepository.isRemoteMode()) {
            Log.d(TAG, "remote storage mode — reminders off")
            return@runCatching Result.success()
        }

        // The med check + daily-summary check both need to read events from
        // storage. If storage isn't open yet we ask WorkManager to retry —
        // the user is still onboarding or the handle hasn't been re-opened
        // post-restart.
        if (!StorageRepository.isOpen()) {
            Log.d(TAG, "storage not open yet — retrying later")
            return@runCatching Result.retry()
        }

        if (Auth.medsRemindersEnabled(ctx)) {
            runCatching { checkMedReminders(ctx) }
                .onFailure { Log.w(TAG, "med reminder check failed", it) }
            runCatching { checkMeasurementReminders(ctx) }
                .onFailure { Log.w(TAG, "measurement reminder check failed", it) }
        }

        if (Auth.dailySummaryEnabled(ctx)) {
            runCatching { checkDailySummary(ctx) }
                .onFailure { Log.w(TAG, "daily summary check failed", it) }
        }

        if (Auth.calendarExportEnabled(ctx)) {
            // TODO: v1.x — write each prescribed dose to the system calendar
            //       via CalendarContract.Events. Requires READ/WRITE_CALENDAR
            //       runtime permissions + a calendar picker in the settings
            //       screen. Out of scope for the v1 notification cut.
            Log.d(TAG, "calendar export enabled but not yet implemented")
        }

        Result.success()
    }.getOrElse { e ->
        Log.w(TAG, "tick failed", e)
        Result.retry()
    }

    /**
     * Fire a reminder for any active medication regimen whose current
     * schedule slot is due/overdue and unsatisfied. Driven by the regimen's
     * real stored `schedule` (cron / anchor) via [Schedule.dueStatus] — not
     * the old `StubData` heuristic. One reminder per slot (dedup-keyed).
     */
    private fun checkMedReminders(ctx: Context) {
        val regimens = activeRegimens() // regimen_id, name, schedule, startMs
        if (regimens.isEmpty()) return

        // Latest non-skipped dose per regimen — by regimen_id and by name,
        // mirroring the Medications screen's matching.
        val doses = StorageRepository.queryEvents(
            EventFilter(eventTypesIn = listOf("medication.taken"), limit = 500L),
        ).getOrNull().orEmpty()
        val lastById = HashMap<String, Long>()
        val lastByName = HashMap<String, Long>()
        doses.forEach { ev ->
            fun ch(p: String) = ev.channels.firstOrNull { it.path == p }?.display
            val skipped = ch("status") == "skipped" || ch("skipped")?.equals("true", true) == true
            if (skipped) return@forEach
            ch("regimen_id")?.takeIf { it.isNotEmpty() }?.let {
                lastById[it] = maxOf(lastById[it] ?: 0L, ev.timestampMs)
            }
            (ch("name") ?: ch("med.name"))?.lowercase()?.takeIf { it.isNotEmpty() }?.let {
                lastByName[it] = maxOf(lastByName[it] ?: 0L, ev.timestampMs)
            }
        }

        val now = System.currentTimeMillis()
        val seen = loadDedupSet(ctx).toMutableSet()
        var changed = false

        regimens.forEach { r ->
            val sched = Schedule.parse(r.schedule)
            if (sched is Schedule.Unscheduled) return@forEach
            val last = listOfNotNull(lastById[r.regimenId], lastByName[r.name.lowercase()]).maxOrNull()
            // currentSlotMs is non-null only when a slot is due/overdue and
            // unsatisfied — handles cron, anchor, and floating intervals.
            val slot = sched.currentSlotMs(last, now) ?: return@forEach
            // Only nag for slots after the regimen began, so a med added at 3pm
            // with a "daily 8am" schedule isn't instantly "overdue".
            if (slot < r.startMs) return@forEach

            val key = "med_${r.regimenId}_${slot / TimeUnit.HOURS.toMillis(1L)}"
            if (seen.contains(key)) return@forEach
            NotificationCenter.append(
                ctx = ctx,
                entry = NotificationCenter.NotificationEntry(
                    id = key,
                    timestampMs = now,
                    title = "Time to take ${r.name}",
                    body = if (last != null) "Last dose ${formatRelative(now - last)}." else "Scheduled dose due.",
                    kind = NotificationCenter.Kind.MED_REMINDER,
                    actionRoute = "log/medication",
                ),
            )
            seen.add(key)
            changed = true
        }

        if (changed) saveDedupSet(ctx, seen)
    }

    /**
     * Mirror of [checkMedReminders] for measurement watches: fire when a
     * watch's schedule slot is due/overdue and no reading has been logged
     * for it. Driven by the watch's stored `schedule`.
     */
    private fun checkMeasurementReminders(ctx: Context) {
        val watches = activeWatches() // metric, label, schedule, startMs
        if (watches.isEmpty()) return

        val now = System.currentTimeMillis()
        val seen = loadDedupSet(ctx).toMutableSet()
        var changed = false

        watches.forEach { w ->
            val sched = Schedule.parse(w.schedule)
            if (sched is Schedule.Unscheduled) return@forEach
            val lastReading = StorageRepository.queryEvents(
                EventFilter(eventTypesIn = listOf("measurement.${w.metric}"), limit = 1L),
            ).getOrNull()?.firstOrNull()?.timestampMs
            val slot = sched.currentSlotMs(lastReading, now) ?: return@forEach
            if (slot < w.startMs) return@forEach

            val key = "watch_${w.metric}_${slot / TimeUnit.HOURS.toMillis(1L)}"
            if (seen.contains(key)) return@forEach
            val label = w.label ?: w.metric.replace('_', ' ')
            NotificationCenter.append(
                ctx = ctx,
                entry = NotificationCenter.NotificationEntry(
                    id = key,
                    timestampMs = now,
                    title = "Time to measure $label",
                    body = "A scheduled reading is due.",
                    kind = NotificationCenter.Kind.MED_REMINDER,
                    actionRoute = "log/measurement",
                ),
            )
            seen.add(key)
            changed = true
        }

        if (changed) saveDedupSet(ctx, seen)
    }

    // ---- tracked-item loaders (via the MCP tool dispatch) ----------------

    private data class RegimenRef(val regimenId: String, val name: String, val schedule: String?, val startMs: Long)
    private data class WatchRef(val metric: String, val label: String?, val schedule: String?, val startMs: Long)

    private fun activeRegimens(): List<RegimenRef> {
        val raw = StorageRepository.executeToolJson("list_active_regimens", "{}").getOrNull() ?: return emptyList()
        return runCatching {
            val arr = JSONObject(raw).optJSONArray("regimens")
            (0 until (arr?.length() ?: 0)).mapNotNull { i ->
                val o = arr!!.optJSONObject(i) ?: return@mapNotNull null
                val id = o.optString("regimen_id", "").ifEmpty { return@mapNotNull null }
                // Fall back to the free-text frequency so "weekly" etc. drive
                // reminders even without an explicit machine schedule.
                val sched = o.optString("schedule", "").ifEmpty { o.optString("frequency", "") }
                    .ifEmpty { null }
                RegimenRef(id, o.optString("name", "Medication"), sched, o.optLong("ts_ms", 0L))
            }
        }.getOrDefault(emptyList())
    }

    private fun activeWatches(): List<WatchRef> {
        val raw = StorageRepository.executeToolJson("list_measurement_watches", "{}").getOrNull() ?: return emptyList()
        return runCatching {
            val arr = JSONObject(raw).optJSONArray("watches")
            (0 until (arr?.length() ?: 0)).mapNotNull { i ->
                val o = arr!!.optJSONObject(i) ?: return@mapNotNull null
                val metric = o.optString("metric", "").ifEmpty { return@mapNotNull null }
                WatchRef(metric, o.optString("label", "").ifEmpty { null },
                    o.optString("schedule", "").ifEmpty { null }, o.optLong("ts_ms", 0L))
            }
        }.getOrDefault(emptyList())
    }

    /**
     * Fire one daily-summary entry at or after 21:00 local. Dedup-keyed by
     * yyyy-MM-dd so the same calendar day only generates one summary even
     * across worker ticks at 21:00 / 21:30 / etc.
     */
    private fun checkDailySummary(ctx: Context) {
        val now = Calendar.getInstance()
        if (now.get(Calendar.HOUR_OF_DAY) < SUMMARY_HOUR) return

        val dayKey = "summary_%04d-%02d-%02d".format(
            now.get(Calendar.YEAR),
            now.get(Calendar.MONTH) + 1,
            now.get(Calendar.DAY_OF_MONTH),
        )
        val seen = loadDedupSet(ctx)
        if (seen.contains(dayKey)) return

        // Count today's events. Day boundary is local midnight.
        val startOfDay = (now.clone() as Calendar).apply {
            set(Calendar.HOUR_OF_DAY, 0)
            set(Calendar.MINUTE, 0)
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis
        val count = StorageRepository.countEvents(
            EventFilter(fromMs = startOfDay, toMs = now.timeInMillis, limit = null),
        ).getOrNull() ?: 0L

        NotificationCenter.append(
            ctx = ctx,
            entry = NotificationCenter.NotificationEntry(
                id = dayKey,
                timestampMs = now.timeInMillis,
                title = "Daily summary",
                body = "$count events logged today. Tap to see them.",
                kind = NotificationCenter.Kind.DAILY_SUMMARY,
                actionRoute = "history",
            ),
        )

        saveDedupSet(ctx, seen + dayKey)
    }

    // ---- dedup helpers ---------------------------------------------------

    private fun loadDedupSet(ctx: Context): Set<String> {
        val raw = Auth.securePrefs(ctx)
            .getString(NotificationCenter.PREF_KEY_DEDUP, null) ?: return emptySet()
        return runCatching {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { arr.getString(it) }.toSet()
        }.getOrElse {
            Log.w(TAG, "dedup-set parse failed; resetting", it)
            emptySet()
        }
    }

    private fun saveDedupSet(ctx: Context, set: Set<String>) {
        // Cap at MAX_DEDUP_ENTRIES so the set doesn't grow without bound
        // over a long uninstall-free run. Oldest keys go first; insertion
        // order is preserved by LinkedHashSet so the trim is deterministic.
        val capped: Set<String> = if (set.size <= MAX_DEDUP_ENTRIES) set
        else set.toList().takeLast(MAX_DEDUP_ENTRIES).toSet()
        val arr = JSONArray()
        capped.forEach { arr.put(it) }
        Auth.securePrefs(ctx).edit()
            .putString(NotificationCenter.PREF_KEY_DEDUP, arr.toString())
            .apply()
    }

    /**
     * Coarse human-readable delta for the med-reminder body ("Last dose:
     * 3h 20m ago"). Anything under 1 minute is "just now"; anything under
     * 60 minutes is in minutes; anything beyond is hour-rounded.
     */
    private fun formatRelative(deltaMs: Long): String {
        val mins = deltaMs / 60_000L
        return when {
            mins < 1 -> "just now"
            mins < 60 -> "${mins}m ago"
            else -> {
                val hours = mins / 60
                val remMins = mins % 60
                if (remMins == 0L) "${hours}h ago" else "${hours}h ${remMins}m ago"
            }
        }
    }

    companion object {
        private const val TAG = "OhdRemindersWorker"

        /** Grace period before "overdue" — matches the brief's `+30 min`. */
        private val LATE_GRACE_MS = TimeUnit.MINUTES.toMillis(30L)

        /** Hour-of-day at which the daily summary fires (local time). */
        private const val SUMMARY_HOUR = 21

        /** Cap on the dedup-set so it can't grow forever. */
        private const val MAX_DEDUP_ENTRIES = 256
    }
}
