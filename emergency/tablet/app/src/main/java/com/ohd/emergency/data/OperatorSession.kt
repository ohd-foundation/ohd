package com.ohd.emergency.data

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * Operator-OIDC session state for the paramedic on this tablet.
 *
 * Per `SPEC.md` "Auth model on the tablet":
 *
 *     Operator OIDC for the responder (paramedic) at shift-in.
 *     Standard OAuth2 / OIDC against the operator's IdP. Token lives
 *     in Android EncryptedSharedPreferences / iOS Keychain.
 *
 * v0.x (2026-05-09): backed by `EncryptedSharedPreferences` from
 * `androidx.security:security-crypto`, which transparently wraps every
 * read/write with a Keystore-bound AES-GCM key. The real OIDC flow
 * (OAuth Code + PKCE in a Custom Tab via AppAuth-Android) lands in
 * [OidcManager]; the legacy [stubSignIn] path is kept for tests + as a
 * smoke-test sign-in shape until AppAuth has been wired into Compose.
 *
 * Three pieces of state:
 *  - **Bearer token**: the operator-IdP-issued OIDC bearer for this
 *    paramedic on this device for this shift. The relay validates it
 *    on every OHDC call. Cleared on shift-out / panic-logout.
 *  - **Operator label**: the human-readable label of the operator
 *    organization ("EMS Prague Region — Crew 42"). Sourced from the
 *    OIDC claims at sign-in; cached for the top-bar display so the
 *    Login screen doesn't have to redecode the JWT every render.
 *  - **Responder label**: the human-readable label of the responder
 *    ("Officer Novák"). Same source; same lifetime.
 *
 * Active case grants are NOT stored here. Per `SPEC.md` "Trust boundary":
 *
 *     Active case grant tokens — issued by the patient phone after
 *     break-glass, scoped to one case, expire on case close.
 *     Memory-only; not persisted to disk on the tablet.
 *
 * They live in [CaseVault] (in-memory) and never touch disk.
 */
object OperatorSession {

    private const val TAG = "OhdEmergency.OperatorSession"

    private const val PREF_NAME = "ohd_emergency_state_secure"
    private const val LEGACY_PREF_NAME = "ohd_emergency_state"
    private const val KEY_BEARER = "operator_bearer"
    private const val KEY_REFRESH = "operator_refresh_token"
    private const val KEY_ACCESS_EXPIRES_AT_MS = "operator_access_expires_at_ms"
    private const val KEY_OPERATOR_LABEL = "operator_label"
    private const val KEY_RESPONDER_LABEL = "responder_label"
    private const val KEY_RESPONDER_SUBJECT = "responder_subject"
    private const val KEY_SHIFT_STARTED_AT_MS = "shift_started_at_ms"
    private const val KEY_AUTH_STATE_JSON = "appauth_state_json"
    private const val KEY_RELAY_BASE_URL = "relay_base_url"

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
     * without crashing on first launch — the caller's TODO is then to
     * surface a "secure storage unavailable" warning.
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

    fun isSignedIn(ctx: Context): Boolean = prefs(ctx).getString(KEY_BEARER, null) != null

    fun bearer(ctx: Context): String? = prefs(ctx).getString(KEY_BEARER, null)

    fun refreshToken(ctx: Context): String? = prefs(ctx).getString(KEY_REFRESH, null)

    fun accessExpiresAtMs(ctx: Context): Long? =
        prefs(ctx).getLong(KEY_ACCESS_EXPIRES_AT_MS, 0L).takeIf { it > 0 }

    fun operatorLabel(ctx: Context): String? = prefs(ctx).getString(KEY_OPERATOR_LABEL, null)
    fun responderLabel(ctx: Context): String? = prefs(ctx).getString(KEY_RESPONDER_LABEL, null)
    fun responderSubject(ctx: Context): String? = prefs(ctx).getString(KEY_RESPONDER_SUBJECT, null)
    fun shiftStartedAtMs(ctx: Context): Long? =
        prefs(ctx).getLong(KEY_SHIFT_STARTED_AT_MS, 0L).takeIf { it > 0 }

    /** AppAuth's serialized [AuthState] JSON — used by silent token refresh. */
    fun appAuthStateJson(ctx: Context): String? = prefs(ctx).getString(KEY_AUTH_STATE_JSON, null)

    /**
     * The operator's relay base URL ("https://relay.ems-prague.cz"). Set
     * out-of-band per fleet provisioning (QR-onboarding at shift-in,
     * MDM-pushed config). When null, the [com.ohd.emergency.data.ohdc.OhdcClientFactory]
     * falls through to BuildConfig + dev defaults.
     */
    fun relayBaseUrl(ctx: Context): String? = prefs(ctx).getString(KEY_RELAY_BASE_URL, null)

    /** Persist the operator's relay base URL; called from the onboarding flow. */
    fun setRelayBaseUrl(ctx: Context, url: String?) {
        prefs(ctx).edit().putString(KEY_RELAY_BASE_URL, url).apply()
    }

    /**
     * Persist the result of a successful AppAuth Code + PKCE flow.
     * Called from [OidcManager.handleAuthResult].
     */
    fun signInWithOidc(
        ctx: Context,
        bearer: String,
        refreshToken: String?,
        accessExpiresAtMs: Long?,
        operatorLabel: String,
        responderLabel: String,
        responderSubject: String,
        authStateJson: String?,
    ) {
        prefs(ctx).edit()
            .putString(KEY_BEARER, bearer)
            .putString(KEY_REFRESH, refreshToken)
            .putLong(KEY_ACCESS_EXPIRES_AT_MS, accessExpiresAtMs ?: 0L)
            .putString(KEY_OPERATOR_LABEL, operatorLabel)
            .putString(KEY_RESPONDER_LABEL, responderLabel)
            .putString(KEY_RESPONDER_SUBJECT, responderSubject)
            .putString(KEY_AUTH_STATE_JSON, authStateJson)
            .putLong(KEY_SHIFT_STARTED_AT_MS, System.currentTimeMillis())
            .apply()
    }

    /**
     * Stub sign-in. Kept for development convenience — a paramedic on a
     * dev tablet with no IdP set up can still smoke-test the flow.
     * Real sign-ins go through [OidcManager.startAuthFlow].
     */
    fun stubSignIn(
        ctx: Context,
        operatorLabel: String,
        responderLabel: String,
        responderSubject: String,
    ) {
        signInWithOidc(
            ctx = ctx,
            bearer = "ohde_DEV_STUB_OIDC_${System.currentTimeMillis()}",
            refreshToken = null,
            accessExpiresAtMs = null,
            operatorLabel = operatorLabel,
            responderLabel = responderLabel,
            responderSubject = responderSubject,
            authStateJson = null,
        )
    }

    /**
     * Panic-logout. Per `SPEC.md` "Tablet device-management expectations":
     *
     *     The app does provide a "panic logout" action that drops
     *     in-memory grants and operator OIDC tokens.
     *
     * Clears every persisted credential. Callers are responsible for
     * also calling `CaseVault.clear()` to drop the in-memory grant +
     * cached patient data.
     */
    fun signOut(ctx: Context) {
        prefs(ctx).edit().clear().apply()
    }
}
