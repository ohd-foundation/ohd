package com.ohd.connect.data

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * On-device retention limits chosen by the user via the storage card's
 * "retention limits" dialog (see `RetentionDialog`).
 *
 *  - [maxAgeYears] — keep only entries newer than N years. `null` means
 *    "unlimited" (the default on first run).
 *  - [maxSizeGb]   — cap the on-device DB file at N GB. `null` means
 *    "unlimited".
 *
 * Persisted via [Auth.saveRetentionLimits]. The Health Connect agent and
 * any future GC job read the same values via [Auth.loadRetentionLimits].
 */
data class RetentionLimits(
    val maxAgeYears: Int? = null,
    val maxSizeGb: Int? = null,
)

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
    private const val KEY_RETENTION_MAX_AGE_YEARS = "retention_max_age_years"
    private const val KEY_RETENTION_MAX_SIZE_GB = "retention_max_size_gb"

    // ---- Settings sub-screen prefs ----------------------------------------
    //
    // These keys are read by the screens under
    // `ui/screens/settings/{Food,Forms,Reminders}SettingsScreen.kt` and by
    // the reminder/notification engine (a sibling agent's work) that polls
    // them when deciding what to fire.
    //
    // Beta-only stack: the user wipes data each install, so no migration
    // required — bump these freely.
    private const val KEY_CUSTOM_FORMS_JSON = "custom_forms_v1"
    private const val KEY_CUSTOM_METRICS_JSON = "custom_metrics_v1"
    private const val KEY_FOOD_TARGET_KCAL = "food_target_kcal_int"
    private const val KEY_FOOD_TARGET_CARBS = "food_target_carbs_g_int"
    private const val KEY_FOOD_TARGET_PROTEIN = "food_target_protein_g_int"
    private const val KEY_FOOD_TARGET_FAT = "food_target_fat_g_int"
    private const val KEY_FOOD_OPENFOODFACTS_ENABLED = "food_openfoodfacts_enabled_bool"
    private const val KEY_REMINDERS_MEDS_ENABLED = "reminders_meds_enabled_bool"
    private const val KEY_REMINDERS_DAILY_SUMMARY_ENABLED = "reminders_daily_summary_enabled_bool"
    private const val KEY_REMINDERS_CALENDAR_EXPORT_ENABLED = "reminders_calendar_export_enabled_bool"

    // ---- Home favourites + CORD model -----------------------------------
    //
    // The Home favourites strip (`HomeScreen.kt`) reads/writes a JSON array
    // of `{ label, kind, icon }` rows under [KEY_HOME_FAVOURITES]. The CORD
    // model picker persists the user's selected model name under
    // [KEY_CORD_SELECTED_MODEL] (default = "claude-3.5-sonnet").
    private const val KEY_HOME_FAVOURITES = "home_favourites_v1"
    private const val KEY_CORD_SELECTED_MODEL = "cord_selected_model"

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

    // ---- Retention limits --------------------------------------------------
    //
    // The on-device storage card lets the user cap retention by (a) max age
    // of stored data in years and (b) max DB file size in GB. Both are
    // optional — `null` means "unlimited". Encoded as -1 in prefs because
    // SharedPreferences has no nullable Int.

    /**
     * Load the persisted retention limits. Defaults to both-unlimited if
     * unset (i.e. on first run before the user touches the dialog).
     */
    fun loadRetentionLimits(ctx: Context): RetentionLimits {
        val p = prefs(ctx)
        val age = p.getInt(KEY_RETENTION_MAX_AGE_YEARS, -1).takeIf { it > 0 }
        val size = p.getInt(KEY_RETENTION_MAX_SIZE_GB, -1).takeIf { it > 0 }
        return RetentionLimits(maxAgeYears = age, maxSizeGb = size)
    }

    /**
     * Persist the user's chosen retention limits. `null` for either field
     * is stored as `-1`, which `loadRetentionLimits` reads back as `null`.
     */
    fun saveRetentionLimits(ctx: Context, limits: RetentionLimits) {
        prefs(ctx).edit()
            .putInt(KEY_RETENTION_MAX_AGE_YEARS, limits.maxAgeYears ?: -1)
            .putInt(KEY_RETENTION_MAX_SIZE_GB, limits.maxSizeGb ?: -1)
            .apply()
    }

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

    // =========================================================================
    // Settings sub-screen accessors
    //
    // Forms, Food, Reminders settings screens each persist a small bag of
    // primitives here. The notification-engine + future Food agent read the
    // same keys when deciding what to render / fire.
    // =========================================================================

    /** Raw JSON blob backing [FormsSettingsScreen]. `null` until first save. */
    fun customFormsJson(ctx: Context): String? =
        prefs(ctx).getString(KEY_CUSTOM_FORMS_JSON, null)

    fun saveCustomFormsJson(ctx: Context, json: String) {
        prefs(ctx).edit().putString(KEY_CUSTOM_FORMS_JSON, json).apply()
    }

    /**
     * Raw JSON blob backing the "Custom measurements" section of
     * [FormsSettingsScreen]. Shape:
     *
     *     {
     *       "metrics": [
     *         {
     *           "namespace": "custom",
     *           "name": "ankle_swelling",
     *           "description": "Ankle swelling",
     *           "value_type": "real" | "int" | "text",
     *           "unit": "cm" | null
     *         }, ...
     *       ]
     *     }
     *
     * v1 is **app-side metadata only** — the storage core doesn't accept
     * runtime-registered event types yet, so we surface these for the user
     * to manage but don't auto-allowlist them server-side. See
     * `spec/registry/metrics.toml` for the canonical (shippable) registry.
     */
    fun customMetricsJson(ctx: Context): String? =
        prefs(ctx).getString(KEY_CUSTOM_METRICS_JSON, null)

    fun saveCustomMetricsJson(ctx: Context, json: String) {
        prefs(ctx).edit().putString(KEY_CUSTOM_METRICS_JSON, json).apply()
    }

    /** Daily nutrition targets. Defaults match the v1 OhdNutriGauge constants. */
    data class FoodTargets(
        val kcal: Int = 2000,
        val carbsG: Int = 250,
        val proteinG: Int = 75,
        val fatG: Int = 65,
    )

    fun loadFoodTargets(ctx: Context): FoodTargets {
        val p = prefs(ctx)
        return FoodTargets(
            kcal = p.getInt(KEY_FOOD_TARGET_KCAL, 2000),
            carbsG = p.getInt(KEY_FOOD_TARGET_CARBS, 250),
            proteinG = p.getInt(KEY_FOOD_TARGET_PROTEIN, 75),
            fatG = p.getInt(KEY_FOOD_TARGET_FAT, 65),
        )
    }

    fun saveFoodTargets(ctx: Context, t: FoodTargets) {
        prefs(ctx).edit()
            .putInt(KEY_FOOD_TARGET_KCAL, t.kcal)
            .putInt(KEY_FOOD_TARGET_CARBS, t.carbsG)
            .putInt(KEY_FOOD_TARGET_PROTEIN, t.proteinG)
            .putInt(KEY_FOOD_TARGET_FAT, t.fatG)
            .apply()
    }

    /** OpenFoodFacts lookups toggle. Off by default — integration unshipped in v1. */
    fun openFoodFactsEnabled(ctx: Context): Boolean =
        prefs(ctx).getBoolean(KEY_FOOD_OPENFOODFACTS_ENABLED, false)

    fun setOpenFoodFactsEnabled(ctx: Context, enabled: Boolean) {
        prefs(ctx).edit().putBoolean(KEY_FOOD_OPENFOODFACTS_ENABLED, enabled).apply()
    }

    /** Medication reminder notifications. On by default. */
    fun medsRemindersEnabled(ctx: Context): Boolean =
        prefs(ctx).getBoolean(KEY_REMINDERS_MEDS_ENABLED, true)

    fun setMedsRemindersEnabled(ctx: Context, enabled: Boolean) {
        prefs(ctx).edit().putBoolean(KEY_REMINDERS_MEDS_ENABLED, enabled).apply()
    }

    /** Daily summary notification. Off by default. */
    fun dailySummaryEnabled(ctx: Context): Boolean =
        prefs(ctx).getBoolean(KEY_REMINDERS_DAILY_SUMMARY_ENABLED, false)

    fun setDailySummaryEnabled(ctx: Context, enabled: Boolean) {
        prefs(ctx).edit().putBoolean(KEY_REMINDERS_DAILY_SUMMARY_ENABLED, enabled).apply()
    }

    /** Mirror med doses into the phone calendar. Off by default. */
    fun calendarExportEnabled(ctx: Context): Boolean =
        prefs(ctx).getBoolean(KEY_REMINDERS_CALENDAR_EXPORT_ENABLED, false)

    fun setCalendarExportEnabled(ctx: Context, enabled: Boolean) {
        prefs(ctx).edit().putBoolean(KEY_REMINDERS_CALENDAR_EXPORT_ENABLED, enabled).apply()
    }

    /** True iff at least one of the three reminder toggles is on. */
    fun remindersAnyEnabled(ctx: Context): Boolean =
        medsRemindersEnabled(ctx) ||
            dailySummaryEnabled(ctx) ||
            calendarExportEnabled(ctx)

    /**
     * Public accessor for the encrypted prefs handle Auth uses internally.
     *
     * Used by [NotificationCenter] to persist its JSON log + dedup set under
     * the same `ohd_connect_secure` file. Keeping notification state in the
     * same prefs file means a single `clear()` (e.g. the SmokeTest setUp)
     * also drops the notification log, matching the beta "wipe data each
     * cycle" workflow.
     */
    fun securePrefs(ctx: Context): SharedPreferences = prefs(ctx)

    // =========================================================================
    // Home favourites + CORD model
    //
    // Tiny convenience getters/setters. The favourites blob is hand-rolled
    // JSON (one object per line wrapped in a JSON array) so we don't have to
    // pull in a JSON library just for this. The CORD model is a plain string.
    // =========================================================================

    /**
     * Load the persisted home favourites blob. Returns `null` when the user
     * hasn't customised the strip — callers fall back to the static default
     * pair (Glucose + Blood pressure).
     */
    fun homeFavouritesJson(ctx: Context): String? =
        prefs(ctx).getString(KEY_HOME_FAVOURITES, null)

    fun saveHomeFavouritesJson(ctx: Context, json: String) {
        prefs(ctx).edit().putString(KEY_HOME_FAVOURITES, json).apply()
    }

    /** Default value matches the chip rendered in the CORD top-bar. */
    fun cordSelectedModel(ctx: Context): String =
        prefs(ctx).getString(KEY_CORD_SELECTED_MODEL, null) ?: "claude-haiku-4-5"

    fun saveCordSelectedModel(ctx: Context, model: String) {
        prefs(ctx).edit().putString(KEY_CORD_SELECTED_MODEL, model).apply()
    }

    // =========================================================================
    // BUGFIX BLOCK — storage option preference (bug #4 — Storage settings
    // didn't reflect the user's onboarding choice). The key is stored as a
    // plain enum-name string so values are stable across app upgrades; future
    // additions just append. See SmokeTest / wipe-data-each-install policy
    // for migration concerns (we drop the key when unknown).
    //
    // Read by:  OnboardingStorageScreen (initial selection on re-entry),
    //           StorageSettingsScreen (pre-selected card in Settings),
    //           MainActivity (persist on first-run continue).
    // =========================================================================
    private const val KEY_STORAGE_OPTION = "storage_option_v1"

    /**
     * Persist the user's storage choice (e.g. `"OnDevice"`). Pass the enum
     * `name` directly — both `_shared.StorageOption` and `settings.StorageOption`
     * share the same four `name` values so either side can write/read.
     */
    fun saveStorageOption(ctx: Context, optionName: String) {
        prefs(ctx).edit().putString(KEY_STORAGE_OPTION, optionName).apply()
    }

    /**
     * Load the persisted storage choice. Returns the [defaultName] when none
     * is set (first run, freshly-wiped cache) so callers don't have to deal
     * with `null`.
     */
    fun loadStorageOption(ctx: Context, defaultName: String = "OnDevice"): String =
        prefs(ctx).getString(KEY_STORAGE_OPTION, null) ?: defaultName

    // =========================================================================
    // CORD provider API keys + stub toggle
    //
    // Read/written by `ui/screens/settings/CordSettingsScreen.kt`. The three
    // provider keys are stored in the same EncryptedSharedPreferences file as
    // the OIDC tokens, so they inherit the Keystore-wrapped AES-256-GCM
    // protection; nothing further needs doing at the storage layer.
    //
    // `provider` is a stable lowercase string — one of "anthropic", "openai",
    // "gemini". Unknown values resolve to a sentinel key the call sites can
    // safely write to, but we still log so an obvious typo surfaces in the
    // adb output.
    // =========================================================================
    private const val KEY_CORD_API_KEY_ANTHROPIC = "cord_api_key_anthropic"
    private const val KEY_CORD_API_KEY_OPENAI = "cord_api_key_openai"
    private const val KEY_CORD_API_KEY_GEMINI = "cord_api_key_gemini"
    private const val KEY_CORD_STUB_RESPONSES = "cord_stub_responses_enabled_bool"

    /**
     * Map a stable provider string to its prefs key. Falls back to a
     * "unknown" bucket so we don't crash when an unexpected token shows up
     * (e.g. a future provider added to the picker but not here yet).
     */
    private fun providerKey(provider: String): String = when (provider.lowercase()) {
        "anthropic" -> KEY_CORD_API_KEY_ANTHROPIC
        "openai" -> KEY_CORD_API_KEY_OPENAI
        "gemini" -> KEY_CORD_API_KEY_GEMINI
        else -> {
            Log.w(TAG, "Unknown CORD provider token: $provider")
            "cord_api_key_unknown_$provider"
        }
    }

    /** Persist a provider API key. Empty strings clear the slot. */
    fun saveCordApiKey(ctx: Context, provider: String, key: String) {
        val k = providerKey(provider)
        prefs(ctx).edit().apply {
            if (key.isEmpty()) remove(k) else putString(k, key)
        }.apply()
    }

    /** Load a provider API key. Returns `""` when unset so call sites can skip null checks. */
    fun loadCordApiKey(ctx: Context, provider: String): String =
        prefs(ctx).getString(providerKey(provider), null).orEmpty()

    /** Convenience predicate used by the "Set / Not configured" status pill. */
    fun isCordApiKeySet(ctx: Context, provider: String): Boolean =
        loadCordApiKey(ctx, provider).isNotEmpty()

    /**
     * Persist the "stub responses" toggle. Default is `true` until the real
     * provider HTTP wiring lands — the chat screen still echoes canned
     * responses even when an API key has been set, until the user explicitly
     * flips this off.
     */
    fun setCordStubResponses(ctx: Context, enabled: Boolean) {
        prefs(ctx).edit().putBoolean(KEY_CORD_STUB_RESPONSES, enabled).apply()
    }

    fun cordStubResponsesEnabled(ctx: Context): Boolean {
        // If any provider key is set, stub mode is off unless the user has
        // explicitly turned it on. Without keys, stub mode is on by default
        // so the chat still produces output the user can sanity-check.
        val anyKeySet = isCordApiKeySet(ctx, "anthropic") ||
            isCordApiKeySet(ctx, "openai") ||
            isCordApiKeySet(ctx, "gemini")
        return prefs(ctx).getBoolean(KEY_CORD_STUB_RESPONSES, !anyKeySet)
    }
}
