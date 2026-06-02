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
 *  - On-device storage not opened → [Result.retry], so the next firing
 *    picks the work up after the user finishes onboarding (the SQLCipher
 *    key isn't reachable from the worker context).
 *  - Health Connect not installed / permissions revoked → [Result.success]
 *    with zero ingested events; the next firing simply does nothing again.
 *  - Any other exception escapes [syncFromHealthConnect] → caught here and
 *    returned as [Result.retry] so transient failures don't abort the
 *    schedule. WorkManager applies its own exponential backoff between
 *    retries.
 *
 * Remote storage mode: the worker syncs through the remote backend too.
 * It used to skip cleanly because each [syncFromHealthConnect] record cost
 * one network round-trip — a typical HC backfill would have been thousands
 * of RPCs against `storage.ohd.dev`. The bulk-`PutEvents` work in beta57
 * collapses those into ~tens of batched calls, so periodic sync is back
 * on the table on OHD Cloud.
 */
class HealthConnectSyncWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {

    override suspend fun doWork(): Result = runCatching {
        StorageRepository.init(applicationContext)
        // Remote storage doesn't need a SQLCipher key, so the worker can
        // bring the backend up itself on a cold-process firing. (For
        // on-device storage the key lives in MainActivity's setup path,
        // so the worker can't open it here — it retries until the user
        // opens the app and finishes onboarding.)
        if (!StorageRepository.isOpen() && StorageRepository.isRemoteMode()) {
            StorageRepository.openOrCreate("").getOrThrow()
        }
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
