package com.ohd.connect.data

import android.content.Context
import android.content.SharedPreferences
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import java.util.concurrent.TimeUnit

/**
 * Schedules / cancels the periodic Health Connect sync.
 *
 * The work is unique by [WORK_NAME] so calling [enable] multiple times is
 * idempotent — WorkManager keeps the existing schedule via
 * `ExistingPeriodicWorkPolicy.KEEP`. To force a re-pickup of, say, a new
 * interval, [disable] then [enable].
 *
 * 15 minutes is Android's hard floor for `PeriodicWorkRequest`. Going
 * lower would silently round up; going higher saves battery but delays
 * fresh data on Home. Health Connect itself is cheap to query — the
 * `lastSyncMs` cursor in [HealthConnectPrefs] keeps each run delta-only.
 */
object HealthConnectScheduler {

    const val WORK_NAME = "ohd-health-connect-sync"

    private const val PREFS_NAME = "ohd_connect_state"
    private const val KEY_AUTO_SYNC_ENABLED = "hc_auto_sync_enabled"

    /**
     * Whether the user has opted into background sync. Defaults to **true**
     * once permissions are granted — most users want their watch / scale
     * data flowing in by default. Toggle from Settings → Health Connect.
     */
    fun isEnabled(ctx: Context): Boolean =
        prefs(ctx).getBoolean(KEY_AUTO_SYNC_ENABLED, true)

    fun setEnabled(ctx: Context, enabled: Boolean) {
        prefs(ctx).edit().putBoolean(KEY_AUTO_SYNC_ENABLED, enabled).apply()
        if (enabled) enable(ctx) else disable(ctx)
    }

    /** Enqueue the periodic worker (unique, additive). */
    fun enable(ctx: Context) {
        val request = PeriodicWorkRequestBuilder<HealthConnectSyncWorker>(
            repeatInterval = 15,
            repeatIntervalTimeUnit = TimeUnit.MINUTES,
        )
            .setConstraints(
                Constraints.Builder()
                    // Health Connect itself is on-device but our sync writes
                    // into local storage which is also offline. We don't
                    // need network. We do want to avoid waking up on a
                    // critically-low battery — `setRequiresBatteryNotLow`
                    // gives WorkManager licence to skip until charged.
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
     * Call once after first-run / cold-start to honour the persisted
     * `hc_auto_sync_enabled` flag without forcing the user to toggle it
     * each install. Safe to call multiple times.
     */
    fun applyPersistedPreference(ctx: Context) {
        if (isEnabled(ctx)) enable(ctx) else disable(ctx)
    }

    private fun prefs(ctx: Context): SharedPreferences =
        ctx.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
