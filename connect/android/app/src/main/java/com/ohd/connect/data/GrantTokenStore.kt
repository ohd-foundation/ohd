package com.ohd.connect.data

import android.content.Context

/**
 * On-device store for the bearer tokens minted by `OhdcService.CreateGrant`.
 *
 * Why this exists: the storage server only returns the raw `ohdg_…` bearer
 * once — at CreateGrant time. Subsequent `ListGrants` responses carry only
 * the grant row (ULID, label, scope, timestamps) without the bearer
 * (returning it would let any read-side surface impersonate the grantee).
 * The share-link UI on [com.ohd.connect.ui.screens.ShareDetailScreen]
 * needs the bearer to produce a working `ohd://share/cloud?token=…` link;
 * without persisting it locally we had to fall back to embedding the
 * grant ULID, which is not a bearer — CORD then hit storage's `/mcp`
 * with a non-token and storage rejected it with
 * `{"code":-32000,"message":"auth: unauthenticated"}`. Day-one
 * regression for any share that wasn't manually re-issued.
 *
 * Persistence: same Keystore-wrapped `EncryptedSharedPreferences` file
 * as the rest of [Auth] (we go through [Auth.securePrefs] so we
 * inherit the AES-256-GCM master key). Each grant ULID maps to one
 * bearer string; re-issuing replaces it.
 */
object GrantTokenStore {

    private const val KEY_PREFIX = "grant_token_v1_"

    fun save(ctx: Context, grantUlid: String, token: String) {
        Auth.securePrefs(ctx).edit()
            .putString(KEY_PREFIX + grantUlid, token)
            .apply()
    }

    fun load(ctx: Context, grantUlid: String): String? =
        Auth.securePrefs(ctx).getString(KEY_PREFIX + grantUlid, null)

    /** Remove the bearer for a grant — call from revoke / delete flows. */
    fun remove(ctx: Context, grantUlid: String) {
        Auth.securePrefs(ctx).edit().remove(KEY_PREFIX + grantUlid).apply()
    }
}
