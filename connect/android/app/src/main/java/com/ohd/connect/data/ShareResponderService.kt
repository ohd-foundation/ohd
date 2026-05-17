package com.ohd.connect.data

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import com.ohd.connect.MainActivity

/**
 * Foreground service that hosts the OHD Connect share responders.
 *
 * Implements the durable-host half of `cord/spec/data-link.md` §"The
 * phone-side share responder": the responder "maintains the relay tunnel"
 * for every share with remote access enabled. Previously the responders
 * lived only in [ShareResponders] — a process-scoped registry — so
 * backgrounding the app let Android kill the process, drop the relay
 * tunnel, and leave a doctor / CORD attaching to a dead rendezvous.
 *
 * Hosting the registry inside a started foreground service keeps the
 * process (and therefore the tokio runtimes the responders own) alive for
 * as long as at least one share has remote access on. The service:
 *
 *  - On [start]: opens storage if a cold start landed us here without an
 *    open handle (the same stub-key open path [ShareResponders.wake] uses),
 *    then resumes every persisted responder binding.
 *  - Foregrounds itself with a persistent notification — a dedicated
 *    low-importance channel, text reporting how many connections are
 *    reachable, a tap target that opens the app on the Shares screen, and a
 *    "Stop sharing" action that disables remote access on every share and
 *    stops the service.
 *  - Stops itself (and removes the notification) once no share has remote
 *    access left — see [ShareResponders.deactivate] → [stopIfIdle].
 *
 * Lifecycle ownership:
 *  - [ShareResponders.activate] calls [start] so a freshly-activated share
 *    immediately gets a durable host.
 *  - [ShareResponders.deactivate] calls [stopIfIdle] so disabling the last
 *    remote share tears the service (and notification) down.
 *  - `MainActivity` still calls [ShareResponders.resumeAll] on launch for
 *    the in-foreground case; the service is the additive durable layer and
 *    [start] is idempotent, so the two coexist.
 *
 * Push-wake / suspend-when-idle is an explicitly-later layer and is *not*
 * built here — this service is always-on while any remote share exists.
 */
class ShareResponderService : Service() {

    /**
     * Partial wake lock held while responders are running. The QUIC tunnel
     * keeps itself alive with a 15 s keep-alive PING; if the device dozes
     * (screen off — the common case while the user chats with CORD from a
     * laptop) those timers stop firing and the relay idle-times-out the
     * tunnel after 120 s. A partial wake lock keeps the CPU scheduling the
     * keep-alive so the tunnel survives the gaps between exchanges.
     */
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP_SHARING) {
            // "Stop sharing" notification action — disable remote access on
            // every share, which itself stops this service via stopIfIdle.
            Log.i(TAG, "stop-sharing action received")
            disableAllRemoteShares(applicationContext)
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }

        // Promote to foreground immediately — Android requires
        // startForeground() within ~5 s of a startForegroundService() call.
        goForeground(buildNotification(applicationContext, connectionCount = 0))

        // The share responder hosts a *local* relay tunnel over the on-device
        // storage core. Remote storage mode has no local core and no local
        // relay responder — stop cleanly rather than spinning a foreground
        // service with nothing to host.
        StorageRepository.init(applicationContext)
        if (StorageRepository.isRemoteMode()) {
            Log.i(TAG, "remote storage mode — no local share responder to host; stopping")
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }

        // Cold start may have landed us here with no open storage handle.
        // Open it with the persisted stub key — the same path MainActivity
        // and ShareResponders.wake use — then resume the responders.
        ensureStorageOpen(applicationContext)
        ShareResponders.resumeAll(applicationContext)

        val count = ShareResponders.activeCount()
        if (count == 0) {
            // Nothing actually came up (e.g. every binding was cleared
            // between the start request and now) — don't sit foregrounded
            // with no work to do.
            Log.i(TAG, "no active responders after resume; stopping service")
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }

        // Refresh the notification with the real reachable-connection count.
        goForeground(buildNotification(applicationContext, count))

        // Keep the CPU awake so the tunnel's keep-alive survives doze.
        acquireWakeLock()

        // START_STICKY: if Android kills us under memory pressure, recreate
        // the service (with a null intent) and re-run the resume path.
        return START_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
        releaseWakeLock()
        Log.i(TAG, "ShareResponderService destroyed")
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG).apply {
            // Not reference-counted: onStartCommand may run more than once
            // (START_STICKY recreate, a second share activated) — acquire
            // must stay idempotent so a single release in onDestroy frees it.
            setReferenceCounted(false)
            acquire()
        }
    }

    private fun releaseWakeLock() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
    }

    private fun stopForegroundCompat() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
    }

    companion object {
        private const val TAG = "OhdConnect.ShareSvc"

        private const val NOTIFICATION_ID = 0xC0_2D
        private const val CHANNEL_ID = "ohd_share_responder"
        private const val CHANNEL_NAME = "Health data sharing"
        private const val CHANNEL_DESCRIPTION =
            "Shown while OHD Connect keeps your shared health data reachable."

        /** Internal action: the notification's "Stop sharing" button. */
        private const val ACTION_STOP_SHARING = "com.ohd.connect.action.STOP_SHARING"

        /** Wake-lock tag — `package:purpose`, the convention logcat expects. */
        private const val WAKE_LOCK_TAG = "ohd:share-responder"

        /**
         * Start (or, if already running, re-trigger the resume path on) the
         * share-responder foreground service. Idempotent — safe to call from
         * [ShareResponders.activate] every time a share is activated.
         *
         * Uses `startForegroundService` on Android O+ so the service may
         * promote itself to foreground from a background caller.
         */
        fun start(ctx: Context) {
            val intent = Intent(ctx, ShareResponderService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                ctx.startForegroundService(intent)
            } else {
                ctx.startService(intent)
            }
        }

        /**
         * Stop the service iff no share has remote access left — called from
         * [ShareResponders.deactivate]. When the last remote share is turned
         * off the service stops and its notification disappears, so a phone
         * with nothing shared shows no persistent notification.
         */
        fun stopIfIdle(ctx: Context) {
            if (ShareResponders.activeCount() == 0) {
                ctx.stopService(Intent(ctx, ShareResponderService::class.java))
            }
        }

        /**
         * Open storage with the persisted stub key when a cold start brought
         * the service up without an open handle. Mirrors
         * [ShareResponders.wake]'s open path; best-effort.
         */
        private fun ensureStorageOpen(ctx: Context) {
            StorageRepository.init(ctx)
            if (StorageRepository.isOpen()) return
            val opened = if (StorageRepository.isInitialised()) {
                StorageRepository.open("00".repeat(32))
            } else {
                Result.failure(IllegalStateException("storage not initialised"))
            }
            opened.onFailure {
                Log.w(TAG, "could not open storage; responders cannot resume", it)
            }
        }

        /**
         * Disable remote access on every grant — the "Stop sharing" action.
         * Each [ShareResponders.deactivate] tears down one responder; once
         * the last is gone the service is idle.
         */
        private fun disableAllRemoteShares(ctx: Context) {
            ensureStorageOpen(ctx)
            val grantUlids = StorageRepository.listGrants(includeRevoked = true)
                .getOrDefault(emptyList())
                .map { it.ulid }
            for (ulid in grantUlids) {
                if (ShareResponders.binding(ctx, ulid) != null) {
                    runCatching { ShareResponders.deactivate(ctx, ulid) }
                        .onFailure { Log.w(TAG, "deactivate failed for $ulid", it) }
                }
            }
        }

        private fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val channel = NotificationChannel(
                CHANNEL_ID,
                CHANNEL_NAME,
                // Low importance: this is an ambient status notification, not
                // an alert — no sound, no heads-up. It is, however,
                // mandatory (a foreground service must show a notification).
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = CHANNEL_DESCRIPTION
                setShowBadge(false)
            }
            val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.createNotificationChannel(channel)
        }

        private fun buildNotification(ctx: Context, connectionCount: Int): Notification {
            ensureChannel(ctx)

            // Tapping the notification opens the app on the Shares screen.
            val openIntent = Intent(ctx, MainActivity::class.java).apply {
                action = Intent.ACTION_MAIN
                addCategory(Intent.CATEGORY_LAUNCHER)
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
                putExtra(MainActivity.EXTRA_START_ROUTE, "shares")
            }
            val contentPi = PendingIntent.getActivity(
                ctx,
                0,
                openIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )

            // "Stop sharing" action — disables remote sharing + stops the
            // service. Routed back into this service via onStartCommand.
            val stopIntent = Intent(ctx, ShareResponderService::class.java).apply {
                action = ACTION_STOP_SHARING
            }
            val stopPi = PendingIntent.getService(
                ctx,
                1,
                stopIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )

            val body = if (connectionCount == 1) {
                "Sharing health data — 1 connection reachable"
            } else {
                "Sharing health data — $connectionCount connections reachable"
            }

            return NotificationCompat.Builder(ctx, CHANNEL_ID)
                // No dedicated asset yet — reuse the framework's stock glyph,
                // matching NotificationCenter's fallback choice.
                .setSmallIcon(android.R.drawable.ic_menu_share)
                .setContentTitle("OHD Connect")
                .setContentText(body)
                .setContentIntent(contentPi)
                .setOngoing(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setCategory(NotificationCompat.CATEGORY_SERVICE)
                .addAction(0, "Stop sharing", stopPi)
                .build()
        }
    }

    /**
     * Promote this service to the foreground with [notification].
     *
     * On Android 10+ the `foregroundServiceType` must be passed at runtime
     * and must match the manifest `<service>` declaration. We use
     * `CONNECTED_DEVICE`: the responder holds a persistent QUIC tunnel to a
     * relay / remote consumer (the doctor's device). `dataSync` would be
     * wrong — its ~6 h cap kills an always-on responder; `connectedDevice`
     * is the closest always-on networked-peer type.
     */
    private fun goForeground(notification: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }
}
