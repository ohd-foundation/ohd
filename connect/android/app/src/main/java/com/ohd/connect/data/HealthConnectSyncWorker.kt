package com.ohd.connect.data

import android.content.Context
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

/**
 * Periodic worker that pulls fresh records from Android Health Connect and
 * appends them to OHD storage as `measurement.*` / `activity.*` events.
 *
 * The actual sync logic lives in [syncFromHealthConnect]; the worker is a
 * thin shell around it so the job survives process death and reboots
 * (WorkManager persists the request through `JobScheduler`).
 *
 * Failure handling:
 *  - Storage not opened → [Result.retry], so the next firing picks the work
 *    up after the user finishes onboarding.
 *  - Health Connect not installed / permissions revoked → [Result.success]
 *    with zero ingested events; the next firing simply does nothing again.
 *  - Any other exception escapes [syncFromHealthConnect] → caught here and
 *    returned as [Result.retry] so transient failures don't abort the
 *    schedule. WorkManager applies its own exponential backoff between
 *    retries.
 */
class HealthConnectSyncWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {

    override suspend fun doWork(): Result = runCatching {
        if (!StorageRepository.isOpen()) {
            Log.d(TAG, "storage not open yet — retrying later")
            return@runCatching Result.retry()
        }
        val result = syncFromHealthConnect(applicationContext)
        Log.d(
            TAG,
            "sync ok: ingested=${result.ingested} byType=${result.readByType} " +
                "errors=${result.errors.size}",
        )
        Result.success()
    }.getOrElse { e ->
        Log.w(TAG, "sync failed", e)
        Result.retry()
    }

    companion object {
        private const val TAG = "OhdHCSyncWorker"
    }
}
