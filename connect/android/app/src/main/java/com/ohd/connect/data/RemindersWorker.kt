package com.ohd.connect.data

import android.content.Context
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import com.ohd.connect.ui.screens.StubData
import org.json.JSONArray
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
     * Scan prescribed medications for ones whose next dose is overdue by
     * more than [LATE_GRACE_MS] and fire one reminder per overdue dose
     * window. Dedup-set guards against re-firing the same dose every tick.
     */
    private fun checkMedReminders(ctx: Context) {
        val prescribed = StubData.medications.filter { it.kind == StubData.MedKind.Prescribed }
        if (prescribed.isEmpty()) return

        // Pull the most recent take for each medication. The medication
        // logger writes `medication.taken` events with a `med.name` channel
        // (see `MedicationScreen.kt`); older rows may use `name`. Accept
        // either so this works against pre-redesign data.
        val recent = StorageRepository.queryEvents(
            EventFilter(eventTypesIn = listOf("medication.taken"), limit = 200L),
        ).getOrNull().orEmpty()

        val lastTakenByName = mutableMapOf<String, Long>()
        recent.forEach { ev ->
            val name = ev.channels
                .firstOrNull { it.path == "med.name" || it.path == "name" }
                ?.display
                ?: return@forEach
            val current = lastTakenByName[name]
            if (current == null || ev.timestampMs > current) {
                lastTakenByName[name] = ev.timestampMs
            }
        }

        val now = System.currentTimeMillis()
        val seen = loadDedupSet(ctx).toMutableSet()
        var changed = false

        prescribed.forEach { med ->
            val scheduleHours = med.scheduleHours ?: return@forEach
            val last = lastTakenByName[med.name]
            // If the user has never logged this med, treat now-as-baseline:
            // we don't fire a reminder until at least one dose has been
            // taken — otherwise a brand-new install spams "Time to take
            // Metformin" the moment the worker boots.
            if (last == null) return@forEach

            val nextDue = last + TimeUnit.HOURS.toMillis(scheduleHours.toLong())
            if (now < nextDue + LATE_GRACE_MS) return@forEach

            // Dedup key: med name + hour-rounded next-due timestamp. Reset
            // on the next dose window — so the user gets one reminder per
            // missed dose, not one per worker tick.
            val key = "${med.name}_${nextDue / TimeUnit.HOURS.toMillis(1L)}"
            if (seen.contains(key)) return@forEach

            NotificationCenter.append(
                ctx = ctx,
                entry = NotificationCenter.NotificationEntry(
                    id = "med_$key",
                    timestampMs = now,
                    title = "Time to take ${med.name}",
                    body = "Last dose: ${formatRelative(now - last)}",
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
