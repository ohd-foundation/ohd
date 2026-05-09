package com.ohd.connect.data

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * Persistent storage for the self-session bearer token + first-run flag.
 *
 * Per `connect/SPEC.md` "OIDC self-session flow → Android":
 *
 *     Tokens stored:
 *       Android: EncryptedSharedPreferences + Keystore-wrapped key.
 *
 * v0.x (2026-05-09): backed by `EncryptedSharedPreferences` from
 * `androidx.security:security-crypto`, which transparently wraps every
 * read/write with a Keystore-bound AES-256-GCM master key. The contract
 * is identical to the prior plain-`SharedPreferences` implementation —
 * existing call sites need no changes.
 *
 * Three flows persist tokens here:
 *  - **On-device storage path** — `StorageRepository.openOrCreate(...)`
 *    returns a self-session token via the uniffi
 *    `issue_self_session_token()` call, then `saveSelfSessionToken(...)`
 *    writes it.
 *  - **Remote OIDC path** — [OidcManager.handleAuthResult] calls
 *    [signInWithOidc] with the storage AS's Code+PKCE response.
 *  - **Manual paste-token (legacy fallback)** — Settings can still call
 *    `saveSelfSessionToken(...)` directly.
 */
object Auth {

    private const val TAG = "OhdConnect.Auth"

    private const val PREF_NAME = "ohd_connect_secure"
    private const val LEGACY_PREF_NAME = "ohd_connect_state"
    private const val KEY_TOKEN = "self_session_token"
    private const val KEY_REFRESH = "refresh_token"
    private const val KEY_ACCESS_EXPIRES_AT_MS = "access_expires_at_ms"
    private const val KEY_AUTH_STATE_JSON = "appauth_state_json"
    private const val KEY_FIRST_RUN_DONE = "first_run_done"
    private const val KEY_STORAGE_OPENED_AT_MS = "storage_opened_at_ms"

    @Volatile
    private var cachedPrefs: SharedPreferences? = null

    private fun prefs(ctx: Context): SharedPreferences {
        val existing = cachedPrefs
        if (existing != null) return existing
        synchronized(this) {
            val cached = cachedPrefs
            if (cached != null) return cached
            val fresh = openEncryptedPrefs(ctx)
                ?: ctx.getSharedPreferences(LEGACY_PREF_NAME, Context.MODE_PRIVATE)
            cachedPrefs = fresh
            return fresh
        }
    }

    /**
     * Build the EncryptedSharedPreferences instance backed by a Keystore-bound
     * AES-256-GCM master key. Returns null on failure (e.g. broken Keystore
     * on a corrupted emulator) so we can degrade to plain SharedPreferences
     * without crashing on first launch.
     */
    private fun openEncryptedPrefs(ctx: Context): SharedPreferences? = runCatching {
        val masterKey = MasterKey.Builder(ctx)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            ctx,
            PREF_NAME,
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }.onFailure {
        Log.w(TAG, "EncryptedSharedPreferences unavailable; falling back to plain prefs", it)
    }.getOrNull()

    fun isFirstRun(ctx: Context): Boolean = !prefs(ctx).getBoolean(KEY_FIRST_RUN_DONE, false)

    fun markFirstRunDone(ctx: Context) {
        prefs(ctx).edit().putBoolean(KEY_FIRST_RUN_DONE, true).apply()
    }

    fun saveSelfSessionToken(ctx: Context, token: String) {
        prefs(ctx).edit().putString(KEY_TOKEN, token).apply()
    }

    fun getSelfSessionToken(ctx: Context): String? = prefs(ctx).getString(KEY_TOKEN, null)

    fun clearSelfSessionToken(ctx: Context) {
        prefs(ctx).edit()
            .remove(KEY_TOKEN)
            .remove(KEY_REFRESH)
            .remove(KEY_ACCESS_EXPIRES_AT_MS)
            .remove(KEY_AUTH_STATE_JSON)
            .apply()
    }

    /** When was the storage handle last opened (debug aid for Settings). */
    fun recordStorageOpened(ctx: Context) {
        prefs(ctx).edit().putLong(KEY_STORAGE_OPENED_AT_MS, System.currentTimeMillis()).apply()
    }

    fun storageOpenedAtMs(ctx: Context): Long? =
        prefs(ctx).getLong(KEY_STORAGE_OPENED_AT_MS, 0L).takeIf { it > 0 }

    // ---- OIDC-specific accessors ------------------------------------------

    fun refreshToken(ctx: Context): String? = prefs(ctx).getString(KEY_REFRESH, null)

    fun accessExpiresAtMs(ctx: Context): Long? =
        prefs(ctx).getLong(KEY_ACCESS_EXPIRES_AT_MS, 0L).takeIf { it > 0 }

    /** AppAuth's serialized [AuthState] JSON — used by silent token refresh. */
    fun appAuthStateJson(ctx: Context): String? = prefs(ctx).getString(KEY_AUTH_STATE_JSON, null)

    /**
     * Persist the result of a successful AppAuth Code + PKCE flow
     * against the user's OHD Storage AS. Mirrors the
     * `OperatorSession.signInWithOidc` shape used by emergency/tablet.
     */
    fun signInWithOidc(
        ctx: Context,
        accessToken: String,
        refreshToken: String?,
        accessExpiresAtMs: Long?,
        authStateJson: String?,
    ) {
        prefs(ctx).edit()
            .putString(KEY_TOKEN, accessToken)
            .putString(KEY_REFRESH, refreshToken)
            .putLong(KEY_ACCESS_EXPIRES_AT_MS, accessExpiresAtMs ?: 0L)
            .putString(KEY_AUTH_STATE_JSON, authStateJson)
            .putBoolean(KEY_FIRST_RUN_DONE, true)
            .apply()
    }
}
