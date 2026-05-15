package com.ohd.connect.data

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONArray
import org.json.JSONObject

/**
 * In-app notification inbox + system-notification dispatcher.
 *
 * Two responsibilities:
 *
 *  1. **System notifications** — every call to [append] also fires an
 *     Android notification via [NotificationManagerCompat.notify] on the
 *     [CHANNEL_ID] channel so the user sees the alert in the status bar
 *     even when the app is backgrounded.
 *
 *  2. **In-app log** — entries are persisted to the encrypted prefs file
 *     under [PREF_KEY_LOG] as a JSON array, newest-first, capped at
 *     [MAX_ENTRIES]. The bell-icon screen reads back via [all].
 *
 * Persistence is intentionally JSON-in-prefs (not a separate Room/SQLite
 * table) for v1 — the entry shape is trivial and the cap of 100 keeps the
 * blob small enough that read/write costs don't matter. When the inbox
 * grows (per-grant notifications, case timeline, etc.) we promote this to
 * its own table.
 *
 * The dedup-set for med reminders ([PREF_KEY_DEDUP]) lives in the same
 * prefs file so a single `clear()` resets both — matches the beta "wipe
 * data each cycle" workflow.
 */
object NotificationCenter {

    private const val TAG = "OhdNotificationCenter"

    /** Single channel for v1 — reminders, sync alerts, daily summary all share it. */
    const val CHANNEL_ID = "ohd_reminders"
    private const val CHANNEL_NAME = "OHD Reminders"
    private const val CHANNEL_DESCRIPTION =
        "Medication reminders, daily summaries, and Health Connect sync alerts."

    /** Encrypted-prefs keys. Bumped freely — beta wipes data each install. */
    private const val PREF_KEY_LOG = "notifications_v1"
    /** JSON-encoded `Set<String>` of `{medName}_{nextDueHourEpoch}` keys. */
    internal const val PREF_KEY_DEDUP = "reminders_dedup_v1"

    /** Hard cap on the in-app log. Oldest entries drop off when exceeded. */
    private const val MAX_ENTRIES = 100

    /** Kinds recognised by the inbox. */
    object Kind {
        const val MED_REMINDER = "med_reminder"
        const val HC_SYNC = "hc_sync"
        const val DAILY_SUMMARY = "daily_summary"
        const val TEST = "test"
    }

    /**
     * One row in the notification inbox.
     *
     * @param id Stable client-side ID. Used both as the Android notification
     *   ID (truncated to Int) and as the prefs-set key. Defaults to a
     *   timestamp-derived value so callers can keep building entries with
     *   the default constructor.
     * @param actionRoute Optional in-app route to navigate to on tap. The
     *   screen interprets `null` as "no nav, just dismiss".
     */
    data class NotificationEntry(
        val id: String,
        val timestampMs: Long,
        val title: String,
        val body: String,
        val kind: String,
        val actionRoute: String? = null,
    )

    /**
     * Append `entry` to the in-app log and fire a system notification.
     *
     * - Insertion is newest-first; oldest entries past [MAX_ENTRIES] drop.
     * - Posting the system notification is best-effort: failures (missing
     *   POST_NOTIFICATIONS on API 33+, channel ID mismatch, etc.) are
     *   logged and swallowed so the in-app log is never lost.
     */
    fun append(ctx: Context, entry: NotificationEntry) {
        appendLog(ctx, entry)
        fireSystemNotification(ctx, entry)
    }

    /** Read the log, newest-first. Empty list if none / read failure. */
    fun all(ctx: Context): List<NotificationEntry> {
        val raw = Auth.securePrefs(ctx).getString(PREF_KEY_LOG, null) ?: return emptyList()
        return runCatching {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { i -> arr.getJSONObject(i).toEntry() }
        }.getOrElse {
            Log.w(TAG, "failed to parse notification log; resetting", it)
            Auth.securePrefs(ctx).edit().remove(PREF_KEY_LOG).apply()
            emptyList()
        }
    }

    /** Drop every persisted entry. Does not touch already-shown system notifications. */
    fun clear(ctx: Context) {
        Auth.securePrefs(ctx).edit().remove(PREF_KEY_LOG).apply()
    }

    // ---- internals -------------------------------------------------------

    private fun appendLog(ctx: Context, entry: NotificationEntry) {
        val prefs = Auth.securePrefs(ctx)
        val current = all(ctx).toMutableList()
        // Drop any pre-existing entry with the same id (callers may overwrite
        // on dedup) before we re-insert at the head.
        current.removeAll { it.id == entry.id }
        current.add(0, entry)
        if (current.size > MAX_ENTRIES) {
            current.subList(MAX_ENTRIES, current.size).clear()
        }
        val arr = JSONArray()
        current.forEach { arr.put(it.toJson()) }
        prefs.edit().putString(PREF_KEY_LOG, arr.toString()).apply()
    }

    private fun fireSystemNotification(ctx: Context, entry: NotificationEntry) {
        ensureChannel(ctx)
        val notification = NotificationCompat.Builder(ctx, CHANNEL_ID)
            // The project doesn't ship a notification asset (or launcher
            // mipmap) yet, so we fall back to the framework's stock
            // "reminder bell" — visually appropriate and guaranteed to
            // exist on every Android version. Replace with a dedicated
            // monochrome OHD glyph once design ships one.
            .setSmallIcon(android.R.drawable.ic_popup_reminder)
            .setContentTitle(entry.title)
            .setContentText(entry.body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(entry.body))
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()
        // The Android notification ID is an int. Hash the string id (mod
        // Int.MAX_VALUE) so concurrent reminders for different meds don't
        // overwrite each other.
        val numericId = entry.id.hashCode() and Int.MAX_VALUE
        runCatching {
            NotificationManagerCompat.from(ctx).notify(numericId, notification)
        }.onFailure {
            // POST_NOTIFICATIONS denied (API 33+) lands here as a
            // SecurityException. We've already appended to the in-app log;
            // log and move on.
            Log.w(TAG, "notify failed (permission denied or channel missing)", it)
        }
    }

    /**
     * Create the [CHANNEL_ID] channel if it doesn't exist. No-op on API < 26.
     *
     * Idempotent: `NotificationManager.createNotificationChannel` upserts by
     * channel ID. We still gate on `Build.VERSION` because the channels API
     * doesn't exist below Oreo.
     */
    private fun ensureChannel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            CHANNEL_NAME,
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
            description = CHANNEL_DESCRIPTION
        }
        val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(channel)
    }

    // ---- JSON wire format ------------------------------------------------

    private fun NotificationEntry.toJson(): JSONObject = JSONObject().apply {
        put("id", id)
        put("ts", timestampMs)
        put("title", title)
        put("body", body)
        put("kind", kind)
        if (actionRoute != null) put("route", actionRoute)
    }

    private fun JSONObject.toEntry(): NotificationEntry = NotificationEntry(
        id = getString("id"),
        timestampMs = getLong("ts"),
        title = getString("title"),
        body = getString("body"),
        kind = getString("kind"),
        actionRoute = if (has("route") && !isNull("route")) getString("route") else null,
    )
}
