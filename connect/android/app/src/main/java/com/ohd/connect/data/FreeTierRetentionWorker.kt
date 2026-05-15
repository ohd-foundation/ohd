package com.ohd.connect.data

import android.content.Context
import android.util.Log
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.concurrent.TimeUnit

/**
 * Free-tier retention enforcement.
 *
 * Runs once a day. When the user's [Plan] is [Plan.Free] it soft-deletes
 * every event older than [RETENTION_DAYS] days. Paid users are exempt.
 * The soft-delete sets `events.deleted_at_ms` so standard queries skip the
 * row but operator-side audit can still resolve it; a future GC pass can
 * drop the storage.
 *
 * Idempotent: a second tick with no new old events is a no-op. The
 * notification is dedup'd by `id = ohd_retention_swept_<yyyy-mm-dd>` so we
 * don't spam.
 */
class FreeTierRetentionWorker(
    ctx: Context,
    params: WorkerParameters,
) : CoroutineWorker(ctx, params) {

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val account = OhdAccountStore.load(applicationContext)
        if (account == null) {
            Log.i(TAG, "no account yet; skipping")
            return@withContext Result.success()
        }
        if (account.plan != Plan.Free) {
            Log.i(TAG, "plan=${account.plan}; retention enforcement off")
            return@withContext Result.success()
        }
        val cutoffMs = System.currentTimeMillis() - RETENTION_DAYS * 86_400_000L
        val deleted = StorageRepository
            .softDeleteEventsBefore(cutoffMs)
            .getOrElse { e ->
                Log.w(TAG, "soft-delete failed", e)
                return@withContext Result.retry()
            }
        if (deleted > 0) {
            Log.i(TAG, "swept $deleted events older than $RETENTION_DAYS days")
            val today = java.text.SimpleDateFormat("yyyy-MM-dd", java.util.Locale.US)
                .format(java.util.Date())
            NotificationCenter.append(
                applicationContext,
                NotificationCenter.NotificationEntry(
                    id = "ohd_retention_swept_$today",
                    timestampMs = System.currentTimeMillis(),
                    title = "Free tier — $deleted events trimmed",
                    body = "Events older than $RETENTION_DAYS days were removed (free plan). " +
                        "Upgrade to keep history for longer.",
                    kind = NotificationCenter.Kind.TEST,
                    actionRoute = "settings/profile/plan",
                ),
            )
        }
        Result.success()
    }

    companion object {
        private const val TAG = "OhdFreeTierRetention"
        /** Days of history kept on the free tier. Mirrors `saas/SPEC.md`. */
        const val RETENTION_DAYS = 7L
    }
}

/**
 * Scheduler façade — one-line entry from [com.ohd.connect.MainActivity] +
 * after a plan upgrade / downgrade. Idempotent: re-calling [enable] keeps
 * the existing schedule.
 */
object FreeTierRetentionScheduler {

    const val WORK_NAME = "ohd-free-tier-retention"

    fun enable(ctx: Context) {
        val request = PeriodicWorkRequestBuilder<FreeTierRetentionWorker>(
            repeatInterval = 1,
            repeatIntervalTimeUnit = TimeUnit.DAYS,
        )
            .setConstraints(
                Constraints.Builder()
                    .setRequiredNetworkType(NetworkType.NOT_REQUIRED)
                    .setRequiresBatteryNotLow(true)
                    .build(),
            )
            .setInitialDelay(1, TimeUnit.HOURS)
            .build()
        WorkManager.getInstance(ctx)
            .enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.KEEP, request)
    }

    fun disable(ctx: Context) {
        WorkManager.getInstance(ctx).cancelUniqueWork(WORK_NAME)
    }
}
