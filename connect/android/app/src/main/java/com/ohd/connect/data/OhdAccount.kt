package com.ohd.connect.data

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.security.SecureRandom

/**
 * Local-first OHD account (the "OHD SaaS" identity, before any server has
 * ever heard of the user).
 *
 * On first run the app mints a `profile_ulid` plus a printable recovery code
 * (16 rows × 8 Crockford base32 chars = 640 bits of entropy — more than
 * enough to seed K_recovery once the storage core's `create_with_mnemonic`
 * is wired through uniffi). Both live in [Auth]'s encrypted prefs so they
 * survive app restart but not app uninstall.
 *
 *  - **Plan** — defaults to [Plan.Free] with a 7-day rolling retention
 *    (enforced by [FreeTierRetentionWorker]).
 *  - **Linked identities** — optional OIDC providers the user attached after
 *    install, persisted as `{provider, sub}` rows. The list stays empty
 *    until `api.ohd.dev` exposes the link RPC; the UI surface is in place
 *    so testers see the shape.
 *  - **Recovery acknowledgement** — once the user has tapped through the
 *    [com.ohd.connect.ui.screens.RecoveryCodeScreen] "I saved it" CTA the
 *    nag notification stops firing.
 *
 * Server-side this maps to the (future) tables the user described:
 *
 *  - `profile_ulid 1↔N oidc_identities`
 *  - `profile_ulid 1↔1 current_plan`
 *  - `profile_ulid 1↔N payment_records`
 *
 * Server roundtrip is offline today; everything below works on the device.
 */
data class OhdAccount(
    val profileUlid: String,
    val createdAtMs: Long,
    val plan: Plan,
    val recoveryCode: RecoveryCode,
    val recoveryAcknowledgedAtMs: Long?,
    val linkedIdentities: List<LinkedIdentity>,
)

enum class Plan {
    /** 7-day rolling retention, local-only. */
    Free,
    /** Unlimited retention, sync, recovery delegation. Stubbed until api.ohd.dev ships. */
    Paid,
}

/** One linked OIDC identity. `sub` is opaque; the provider is the issuer host. */
data class LinkedIdentity(
    val provider: String,
    val sub: String,
    val displayLabel: String?,
    val linkedAtMs: Long,
)

/**
 * 16 lines × 8 chars Crockford base32, separated by spaces every 4 chars on
 * the display side. Total 128 chars × 5 bits = 640 bits of entropy. Stored
 * as the line list so the recovery screen can render the grid without
 * having to re-split on render.
 */
data class RecoveryCode(val lines: List<String>) {
    init {
        require(lines.size == ROWS) { "RecoveryCode needs $ROWS rows, got ${lines.size}" }
        require(lines.all { it.length == LINE_LEN }) { "Each row must be $LINE_LEN chars" }
    }

    /** Render one row in the canonical `XXXX XXXX` form. */
    fun formatRow(idx: Int): String = lines[idx].chunked(4).joinToString(" ")

    /** All rows joined by newlines — used for clipboard / share intent. */
    fun toShareString(): String = lines.joinToString("\n") { formatRow(0).let { _ -> it } }
        .let { it } // kept for symmetry; lines.joinToString is enough but we wanted the formatted form
        .let { _ -> lines.indices.joinToString("\n") { i -> formatRow(i) } }

    companion object {
        const val ROWS = 16
        const val LINE_LEN = 8
        /** Crockford base32 alphabet — no 0/O, no 1/I/L confusion. */
        private const val ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

        /** Mint a fresh recovery code from [SecureRandom]. */
        fun generate(rng: SecureRandom = SecureRandom()): RecoveryCode {
            val rows = ArrayList<String>(ROWS)
            repeat(ROWS) {
                val sb = StringBuilder(LINE_LEN)
                repeat(LINE_LEN) { sb.append(ALPHABET[rng.nextInt(ALPHABET.length)]) }
                rows += sb.toString()
            }
            return RecoveryCode(rows)
        }

        /** Round-trip a stored code. Returns null if the JSON is malformed. */
        internal fun fromJson(raw: String?): RecoveryCode? {
            if (raw.isNullOrBlank()) return null
            return runCatching {
                val arr = JSONArray(raw)
                val out = ArrayList<String>(arr.length())
                for (i in 0 until arr.length()) out += arr.getString(i)
                RecoveryCode(out)
            }.getOrNull()
        }

        internal fun toJson(code: RecoveryCode): String = JSONArray(code.lines).toString()
    }
}

/**
 * Account persistence + factory. Kept separate from [Auth] (which is
 * already large) so the two concerns can evolve independently.
 */
object OhdAccountStore {

    private const val KEY_PROFILE_ULID = "ohd_account_profile_ulid"
    private const val KEY_CREATED_AT = "ohd_account_created_at_ms"
    private const val KEY_PLAN = "ohd_account_plan"
    private const val KEY_RECOVERY_CODE_JSON = "ohd_account_recovery_code_json"
    private const val KEY_RECOVERY_ACK_AT = "ohd_account_recovery_ack_at_ms"
    private const val KEY_LINKED_IDENTITIES_JSON = "ohd_account_linked_identities_json"

    /**
     * Returns the persisted account, or `null` on first run. The caller is
     * expected to invoke [mintFree] when this returns `null`.
     */
    fun load(ctx: Context): OhdAccount? {
        val prefs = Auth.securePrefs(ctx)
        val ulid = prefs.getString(KEY_PROFILE_ULID, null) ?: return null
        val createdAt = prefs.getLong(KEY_CREATED_AT, 0L)
        val plan = Plan.entries.firstOrNull { it.name == prefs.getString(KEY_PLAN, null) } ?: Plan.Free
        val code = RecoveryCode.fromJson(prefs.getString(KEY_RECOVERY_CODE_JSON, null))
            ?: return null
        val ackAt = prefs.getLong(KEY_RECOVERY_ACK_AT, 0L).takeIf { it > 0 }
        val linked = parseLinked(prefs.getString(KEY_LINKED_IDENTITIES_JSON, null))
        return OhdAccount(
            profileUlid = ulid,
            createdAtMs = createdAt,
            plan = plan,
            recoveryCode = code,
            recoveryAcknowledgedAtMs = ackAt,
            linkedIdentities = linked,
        )
    }

    /**
     * Mint a Free-tier account: fresh ULID, fresh recovery code, no linked
     * identities, no ack yet. Writes through to encrypted prefs.
     */
    fun mintFree(ctx: Context, now: Long = System.currentTimeMillis()): OhdAccount {
        val ulid = newProfileUlid(now)
        val code = RecoveryCode.generate()
        val account = OhdAccount(
            profileUlid = ulid,
            createdAtMs = now,
            plan = Plan.Free,
            recoveryCode = code,
            recoveryAcknowledgedAtMs = null,
            linkedIdentities = emptyList(),
        )
        writeAll(ctx, account)
        return account
    }

    /** Mark the recovery code as saved (or re-confirmed). */
    fun acknowledgeRecovery(ctx: Context, now: Long = System.currentTimeMillis()) {
        Auth.securePrefs(ctx).edit().putLong(KEY_RECOVERY_ACK_AT, now).apply()
    }

    /** Add a linked OIDC identity. Idempotent on (provider, sub). */
    fun addLinkedIdentity(ctx: Context, identity: LinkedIdentity) {
        val current = load(ctx) ?: return
        val merged = current.linkedIdentities
            .filterNot { it.provider == identity.provider && it.sub == identity.sub }
            .plus(identity)
        writeLinked(ctx, merged)
    }

    /** Remove a linked OIDC identity. */
    fun removeLinkedIdentity(ctx: Context, provider: String, sub: String) {
        val current = load(ctx) ?: return
        writeLinked(ctx, current.linkedIdentities.filterNot { it.provider == provider && it.sub == sub })
    }

    /** Upgrade or downgrade the plan. */
    fun setPlan(ctx: Context, plan: Plan) {
        Auth.securePrefs(ctx).edit().putString(KEY_PLAN, plan.name).apply()
    }

    private fun writeAll(ctx: Context, account: OhdAccount) {
        Auth.securePrefs(ctx).edit().apply {
            putString(KEY_PROFILE_ULID, account.profileUlid)
            putLong(KEY_CREATED_AT, account.createdAtMs)
            putString(KEY_PLAN, account.plan.name)
            putString(KEY_RECOVERY_CODE_JSON, RecoveryCode.toJson(account.recoveryCode))
            putLong(KEY_RECOVERY_ACK_AT, account.recoveryAcknowledgedAtMs ?: 0L)
            putString(KEY_LINKED_IDENTITIES_JSON, encodeLinked(account.linkedIdentities))
        }.apply()
    }

    private fun writeLinked(ctx: Context, identities: List<LinkedIdentity>) {
        Auth.securePrefs(ctx).edit()
            .putString(KEY_LINKED_IDENTITIES_JSON, encodeLinked(identities))
            .apply()
    }

    private fun parseLinked(raw: String?): List<LinkedIdentity> {
        if (raw.isNullOrBlank()) return emptyList()
        return runCatching {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                LinkedIdentity(
                    provider = o.getString("provider"),
                    sub = o.getString("sub"),
                    displayLabel = o.optString("displayLabel").takeIf { it.isNotEmpty() },
                    linkedAtMs = o.optLong("linkedAtMs"),
                )
            }
        }.getOrDefault(emptyList())
    }

    private fun encodeLinked(identities: List<LinkedIdentity>): String {
        val arr = JSONArray()
        identities.forEach {
            val o = JSONObject()
                .put("provider", it.provider)
                .put("sub", it.sub)
                .put("linkedAtMs", it.linkedAtMs)
            it.displayLabel?.let { d -> o.put("displayLabel", d) }
            arr.put(o)
        }
        return arr.toString()
    }

    /**
     * Generate a ULID-shaped string for the profile. We don't have a uniffi
     * ULID minter on Android; the storage core's user_ulid lives separately.
     * Format: 10 chars of Crockford-base32 timestamp (48 bits, ms since
     * epoch) + 16 chars random.
     */
    private fun newProfileUlid(now: Long): String {
        val rng = SecureRandom()
        val alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
        val ts = StringBuilder()
        var n = now
        repeat(10) {
            ts.append(alphabet[(n and 31L).toInt()])
            n = n ushr 5
        }
        ts.reverse()
        val rand = StringBuilder()
        repeat(16) { rand.append(alphabet[rng.nextInt(alphabet.length)]) }
        return ts.toString() + rand.toString()
    }
}
