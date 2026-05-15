package com.ohd.connect.data

import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runInterruptible
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * Thin HTTP client for the OHD SaaS account service (see `saas/SPEC.md`).
 *
 * All calls are coroutine-safe (`runInterruptible(Dispatchers.IO)`), never
 * throw to the caller — they return `Result` and swallow network failures
 * so the app keeps working offline. The user's `profile_ulid` and recovery
 * code are minted locally first; this client opportunistically registers
 * them when network is available so `/v1/account/recover` works from
 * another device.
 *
 * Auth: bearer JWT minted at register / recover / claim time. The token
 * lives in encrypted prefs alongside the rest of [Auth]'s state.
 */
object OhdSaasClient {

    private const val TAG = "OhdSaasClient"
    private const val BASE_URL = "https://api.ohd.dev"

    private const val USER_AGENT = "OHD-Connect/0.1 (Android)"
    private const val CONNECT_TIMEOUT_MS = 4_000
    private const val READ_TIMEOUT_MS = 8_000

    // ---------- Public API ----------

    /**
     * Register or claim a profile. Idempotent on `(profile_ulid, recovery_hash)`.
     * Returns the access token + plan on success.
     */
    suspend fun register(profileUlid: String, recoveryCode: String): Result<RegisterResult> =
        post(
            path = "/v1/account",
            body = JSONObject().apply {
                put("profile_ulid", profileUlid)
                put("recovery_code", recoveryCode)
            },
        ).mapCatching { parseRegister(it) }

    /** Submit a recovery code, receive a new access token. */
    suspend fun recover(recoveryCode: String): Result<RegisterResult> =
        post(
            path = "/v1/account/recover",
            body = JSONObject().apply { put("recovery_code", recoveryCode) },
        ).mapCatching { parseRegister(it) }

    /** "Already have an account?" — find a profile by OIDC sub. */
    suspend fun claimOidc(provider: String, sub: String): Result<RegisterResult> =
        post(
            path = "/v1/account/oidc/claim",
            body = JSONObject().apply {
                put("provider", provider)
                put("sub", sub)
            },
        ).mapCatching { parseRegister(it) }

    /** Current profile + linked identities + plan info. Auth required. */
    suspend fun me(accessToken: String): Result<MeResult> =
        get(path = "/v1/account/me", token = accessToken)
            .mapCatching { parseMe(it) }

    /** Link an OIDC identity to the current profile. */
    suspend fun linkOidc(
        accessToken: String,
        provider: String,
        sub: String,
        displayLabel: String?,
    ): Result<LinkedIdentity> =
        post(
            path = "/v1/account/oidc/link",
            body = JSONObject().apply {
                put("provider", provider)
                put("sub", sub)
                if (displayLabel != null) put("display_label", displayLabel)
            },
            token = accessToken,
        ).mapCatching { parseLinkedIdentity(it) }

    /** Plan info: tier, retention days, max storage. */
    suspend fun currentPlan(accessToken: String): Result<PlanInfo> =
        get(path = "/v1/account/plan", token = accessToken)
            .mapCatching { parsePlanInfo(it) }

    /** Stripe stub for now — returns a URL the app opens in the browser. */
    suspend fun checkout(accessToken: String): Result<String> =
        post(
            path = "/v1/account/plan/checkout",
            body = JSONObject(),
            token = accessToken,
        ).mapCatching { it.getString("checkout_url") }

    /** Liveness — used to gate "online" upsell features. */
    suspend fun healthz(): Result<Unit> = runInterruptible(Dispatchers.IO) {
        runCatching {
            val conn = (URL("$BASE_URL/healthz").openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = CONNECT_TIMEOUT_MS
                readTimeout = READ_TIMEOUT_MS
                setRequestProperty("User-Agent", USER_AGENT)
            }
            try {
                val code = conn.responseCode
                if (code !in 200..299) error("status=$code")
                Unit
            } finally {
                conn.disconnect()
            }
        }
    }

    // ---------- Response shapes ----------

    data class RegisterResult(
        val profileUlid: String,
        val accessToken: String,
        val plan: Plan,
        val createdAt: String,
    )

    data class MeResult(
        val profileUlid: String,
        val plan: Plan,
        val createdAt: String,
        val linkedIdentities: List<LinkedIdentity>,
        val planInfo: PlanInfo,
    )

    data class PlanInfo(
        val plan: Plan,
        val retentionDays: Int,
        val maxStorageMb: Int,
        val sync: Boolean,
    )

    // ---------- Parsers ----------

    private fun parseRegister(o: JSONObject) = RegisterResult(
        profileUlid = o.getString("profile_ulid"),
        accessToken = o.getString("access_token"),
        plan = if (o.optString("plan") == "paid") Plan.Paid else Plan.Free,
        createdAt = o.optString("created_at", ""),
    )

    private fun parseMe(o: JSONObject): MeResult {
        val linked = mutableListOf<LinkedIdentity>()
        val arr = o.optJSONArray("linked_identities") ?: JSONArray()
        for (i in 0 until arr.length()) {
            val row = arr.getJSONObject(i)
            linked += LinkedIdentity(
                provider = row.getString("provider"),
                sub = row.getString("sub"),
                displayLabel = row.optString("display_label").takeIf { it.isNotEmpty() },
                linkedAtMs = 0L, // server's linked_at is ISO; we don't surface it client-side yet
            )
        }
        return MeResult(
            profileUlid = o.getString("profile_ulid"),
            plan = if (o.optString("plan") == "paid") Plan.Paid else Plan.Free,
            createdAt = o.optString("created_at", ""),
            linkedIdentities = linked,
            planInfo = parsePlanInfo(o.getJSONObject("plan_info")),
        )
    }

    private fun parsePlanInfo(o: JSONObject) = PlanInfo(
        plan = if (o.optString("plan") == "paid") Plan.Paid else Plan.Free,
        retentionDays = o.optInt("retention_days", 7),
        maxStorageMb = o.optInt("max_storage_mb", 25),
        sync = o.optBoolean("sync", false),
    )

    private fun parseLinkedIdentity(o: JSONObject) = LinkedIdentity(
        provider = o.getString("provider"),
        sub = o.getString("sub"),
        displayLabel = o.optString("display_label").takeIf { it.isNotEmpty() },
        linkedAtMs = System.currentTimeMillis(),
    )

    // ---------- HTTP ----------

    private suspend fun get(path: String, token: String? = null): Result<JSONObject> =
        runInterruptible(Dispatchers.IO) {
            httpJson("GET", path, body = null, token = token)
        }

    private suspend fun post(
        path: String,
        body: JSONObject,
        token: String? = null,
    ): Result<JSONObject> = runInterruptible(Dispatchers.IO) {
        httpJson("POST", path, body = body, token = token)
    }

    private fun httpJson(
        method: String,
        path: String,
        body: JSONObject?,
        token: String?,
    ): Result<JSONObject> = runCatching {
        val conn = (URL("$BASE_URL$path").openConnection() as HttpURLConnection).apply {
            requestMethod = method
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            instanceFollowRedirects = true
            setRequestProperty("User-Agent", USER_AGENT)
            setRequestProperty("Accept", "application/json")
            if (token != null) setRequestProperty("Authorization", "Bearer $token")
            if (body != null) {
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
            }
        }
        try {
            if (body != null) {
                conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
            if (code !in 200..299) {
                Log.w(TAG, "$method $path → $code: $text")
                error("HTTP $code")
            }
            if (text.isBlank()) JSONObject() else JSONObject(text)
        } finally {
            conn.disconnect()
        }
    }
}

/**
 * Encrypted-prefs slot for the OHD SaaS bearer token. Kept separate from
 * the storage self-session token so they can be revoked / rotated
 * independently.
 */
object OhdSaasTokenStore {
    private const val KEY = "ohd_saas_access_token"
    fun save(ctx: android.content.Context, token: String) {
        Auth.securePrefs(ctx).edit().putString(KEY, token).apply()
    }
    fun load(ctx: android.content.Context): String? = Auth.securePrefs(ctx).getString(KEY, null)
    fun clear(ctx: android.content.Context) {
        Auth.securePrefs(ctx).edit().remove(KEY).apply()
    }
}
