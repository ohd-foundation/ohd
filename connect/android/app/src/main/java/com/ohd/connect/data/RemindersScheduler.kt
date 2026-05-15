package com.ohd.connect.data

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import java.util.concurrent.TimeUnit

/**
 * Schedules / cancels the periodic reminder check.
 *
 * Mirrors [HealthConnectScheduler]'s shape: a single named periodic work
 * request enqueued via [ExistingPeriodicWorkPolicy.KEEP] so calling
 * [enable] multiple times is idempotent.
 *
 * Cadence is 30 minutes — the brief picked it as a sane middle ground.
 * The platform floor for PeriodicWorkRequest is 15 minutes; we don't
 * benefit from running more often (a med dose overdue by 5 minutes still
 * shows up as overdue by 35 minutes on the next tick, which is what the
 * `+30 min` grace window in [RemindersWorker] is for).
 *
 * Unlike Health Connect's scheduler we don't keep our own enabled-flag;
 * the source of truth is the three booleans in [Auth] that the Reminders
 * settings screen writes. [applyPersistedPreference] simply enables the
 * worker when *any* of the three is on and cancels it otherwise.
 */
object RemindersScheduler {

    const val WORK_NAME = "ohd-reminders"

    /** Enqueue the periodic worker (unique, additive). */
    fun enable(ctx: Context) {
        val request = PeriodicWorkRequestBuilder<RemindersWorker>(
            repeatInterval = 30,
            repeatIntervalTimeUnit = TimeUnit.MINUTES,
        )
            .setConstraints(
                Constraints.Builder()
                    // The work reads local prefs + storage — no network.
                    // Battery-not-low matches the Health Connect worker so
                    // we don't drain the user's phone on a half-empty
                    // charge.
                    .setRequiredNetworkType(NetworkType.NOT_REQUIRED)
                    .setRequiresBatteryNotLow(true)
                    .build(),
            )
            .setBackoffCriteria(
                BackoffPolicy.EXPONENTIAL,
                30,
                TimeUnit.SECONDS,
            )
            .build()

        WorkManager.getInstance(ctx).enqueueUniquePeriodicWork(
            WORK_NAME,
            ExistingPeriodicWorkPolicy.KEEP,
            request,
        )
    }

    /** Cancel the periodic worker. Idempotent. */
    fun disable(ctx: Context) {
        WorkManager.getInstance(ctx).cancelUniqueWork(WORK_NAME)
    }

    /**
     * Honour the persisted reminder toggles on cold start. Safe to call
     * multiple times — [ExistingPeriodicWorkPolicy.KEEP] preserves the
     * existing schedule when [enable] is invoked against a live job.
     *
     * Enables iff [Auth.remindersAnyEnabled] is true; disables otherwise.
     */
    fun applyPersistedPreference(ctx: Context) {
        if (Auth.remindersAnyEnabled(ctx)) enable(ctx) else disable(ctx)
    }
}
