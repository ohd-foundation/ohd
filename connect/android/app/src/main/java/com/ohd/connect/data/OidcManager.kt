package com.ohd.connect.data

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import net.openid.appauth.AuthState
import net.openid.appauth.AuthorizationException
import net.openid.appauth.AuthorizationRequest
import net.openid.appauth.AuthorizationResponse
import net.openid.appauth.AuthorizationService
import net.openid.appauth.AuthorizationServiceConfiguration
import net.openid.appauth.ResponseTypeValues
import net.openid.appauth.TokenResponse

/**
 * Self-session OIDC manager for OHD Connect on Android.
 *
 * Mirrors the connect/web + connect/cli OIDC story:
 *
 *  - The **user** signs in against their own OHD Storage instance, which
 *    acts as the OAuth Authorization Server toward the Connect SPA / app
 *    per `spec/docs/design/auth.md` "Browser-based clients".
 *  - The flow is OAuth 2.0 Authorization Code + PKCE in a Custom Tab via
 *    [AppAuth-Android](https://github.com/openid/AppAuth-Android).
 *  - The resulting `ohds_…` access token is persisted via [Auth] (which
 *    now uses `EncryptedSharedPreferences`).
 *
 * **Distinct from emergency/tablet's [OidcManager]**: that one signs the
 * paramedic in against the *operator's* IdP. This one signs the user in
 * against *their own storage instance* — same primitives (AppAuth +
 * EncryptedSharedPreferences), different relationship.
 *
 * The issuer URL is configurable per OHD Storage instance (the user
 * pastes their storage URL on the Setup screen). Discovery happens via
 * `<issuer>/.well-known/oauth-authorization-server` (RFC 8414) with
 * AppAuth's automatic fallback to `/openid-configuration`.
 *
 * Default config values (`BuildConfig.OHD_OIDC_*`) are placeholders so
 * the app builds without a deployment-specific issuer baked in.
 *
 * Threading: AppAuth's [AuthorizationService] is built lazily; every public
 * method is safe to call on the main thread.
 */
object OidcManager {

    private const val TAG = "OhdConnect.OIDC"

    /** Per-config snapshot consumed by [startAuthFlow]. */
    data class Config(
        /** OHD Storage instance URL (acts as the OAuth AS). */
        val storageUrl: String,
        val clientId: String,
        val redirectUri: String,
        val scope: String = "openid offline_access",
        /**
         * Extra query params appended to the AS `/authorize` request. The
         * OHD Cloud path passes `provider=ohd_account` so the storage AS
         * skips its in-browser provider-picker page and 302s straight to
         * `accounts.ohd.dev` — the app already knows which provider the
         * user chose, so the picker is redundant.
         */
        val additionalParams: Map<String, String> = emptyMap(),
    )

    @Volatile
    private var authService: AuthorizationService? = null

    private fun service(ctx: Context): AuthorizationService {
        val existing = authService
        if (existing != null) return existing
        synchronized(this) {
            val cached = authService
            if (cached != null) return cached
            val fresh = AuthorizationService(ctx.applicationContext)
            authService = fresh
            return fresh
        }
    }

    /**
     * Discover the AS metadata and return an Intent that, when launched
     * via [ActivityResultLauncher], pops the storage AS in a Custom Tab.
     */
    fun authIntent(
        ctx: Context,
        config: Config,
        onError: (String) -> Unit,
        onIntent: (Intent) -> Unit,
    ) {
        val issuerUri = Uri.parse(config.storageUrl)
        AuthorizationServiceConfiguration.fetchFromIssuer(
            issuerUri,
        ) { serviceConfig: AuthorizationServiceConfiguration?, ex: AuthorizationException? ->
            if (ex != null || serviceConfig == null) {
                Log.w(TAG, "OIDC discovery failed", ex)
                onError(ex?.errorDescription ?: ex?.message ?: "OIDC discovery failed")
                return@fetchFromIssuer
            }
            val req = AuthorizationRequest.Builder(
                serviceConfig,
                config.clientId,
                ResponseTypeValues.CODE,
                Uri.parse(config.redirectUri),
            )
                .setScope(config.scope)
                .setAdditionalParameters(config.additionalParams)
                .build()
            onIntent(service(ctx).getAuthorizationRequestIntent(req))
        }
    }

    /**
     * Convenience: kick off the flow on a [ComponentActivity] using the
     * pre-registered [launcher].
     */
    fun startAuthFlow(
        activity: ComponentActivity,
        launcher: ActivityResultLauncher<Intent>,
        config: Config,
        onError: (String) -> Unit,
    ) {
        authIntent(activity, config, onError) { intent ->
            launcher.launch(intent)
        }
    }

    /**
     * Process the result of the Custom Tab redirect.
     *
     * On success, persists the `ohds_…` access token + refresh + AppAuth
     * state JSON via [Auth.signInWithOidc] (backed by `EncryptedSharedPreferences`).
     * On failure, surfaces an error string for the SetupScreen to render.
     */
    fun handleAuthResult(
        ctx: Context,
        data: Intent?,
        onComplete: (Result<Unit>) -> Unit,
    ) {
        if (data == null) {
            onComplete(Result.failure(IllegalStateException("OIDC redirect: no data")))
            return
        }
        val resp = AuthorizationResponse.fromIntent(data)
        val ex = AuthorizationException.fromIntent(data)
        if (ex != null || resp == null) {
            Log.w(TAG, "auth response error", ex)
            onComplete(
                Result.failure(
                    IllegalStateException(
                        ex?.errorDescription ?: ex?.message ?: "OIDC auth failed",
                    ),
                ),
            )
            return
        }
        val state = AuthState(resp, null as AuthorizationException?)
        val tokenReq = resp.createTokenExchangeRequest()
        service(ctx).performTokenRequest(tokenReq) { tokenResp: TokenResponse?, tokEx: AuthorizationException? ->
            if (tokEx != null || tokenResp == null) {
                Log.w(TAG, "token exchange error", tokEx)
                onComplete(
                    Result.failure(
                        IllegalStateException(
                            tokEx?.errorDescription ?: tokEx?.message ?: "OIDC token exchange failed",
                        ),
                    ),
                )
                return@performTokenRequest
            }
            state.update(tokenResp, null as AuthorizationException?)
            val accessToken = tokenResp.accessToken
            if (accessToken.isNullOrEmpty()) {
                onComplete(
                    Result.failure(IllegalStateException("OIDC token response missing access_token")),
                )
                return@performTokenRequest
            }
            Auth.signInWithOidc(
                ctx = ctx,
                accessToken = accessToken,
                refreshToken = tokenResp.refreshToken,
                accessExpiresAtMs = tokenResp.accessTokenExpirationTime,
                authStateJson = state.jsonSerializeString(),
            )
            onComplete(Result.success(Unit))
        }
    }

    /**
     * Silently refresh the persisted `ohds_…` access token using the
     * `ohdr_…` refresh token captured by the initial Code + PKCE exchange.
     *
     * Rehydrates AppAuth's [AuthState] from the JSON [Auth] persisted
     * during [handleAuthResult], builds a token-refresh request via
     * [AuthState.createTokenRefreshRequest], and performs it. On success the
     * fresh tokens are written back through [Auth.signInWithOidc] — exactly
     * the same persistence path as the initial exchange.
     *
     * Safe to call on the main thread; the AppAuth callback fires on the
     * main looper. [onComplete] receives [Result.failure] when no AppAuth
     * state is persisted yet or the AS rejects the refresh.
     */
    fun refreshAccessToken(
        ctx: Context,
        onComplete: (Result<Unit>) -> Unit,
    ) {
        val stateJson = Auth.appAuthStateJson(ctx)
        if (stateJson.isNullOrEmpty()) {
            onComplete(
                Result.failure(IllegalStateException("OIDC refresh: no persisted AuthState")),
            )
            return
        }
        val state = runCatching { AuthState.jsonDeserialize(stateJson) }.getOrNull()
        if (state == null) {
            onComplete(
                Result.failure(IllegalStateException("OIDC refresh: corrupt AuthState JSON")),
            )
            return
        }
        val refreshReq = runCatching { state.createTokenRefreshRequest() }.getOrNull()
        if (refreshReq == null) {
            onComplete(
                Result.failure(IllegalStateException("OIDC refresh: no refresh token available")),
            )
            return
        }
        service(ctx).performTokenRequest(refreshReq) { tokenResp: TokenResponse?, tokEx: AuthorizationException? ->
            if (tokEx != null || tokenResp == null) {
                Log.w(TAG, "token refresh error", tokEx)
                onComplete(
                    Result.failure(
                        IllegalStateException(
                            tokEx?.errorDescription ?: tokEx?.message ?: "OIDC token refresh failed",
                        ),
                    ),
                )
                return@performTokenRequest
            }
            state.update(tokenResp, null as AuthorizationException?)
            val accessToken = tokenResp.accessToken
            if (accessToken.isNullOrEmpty()) {
                onComplete(
                    Result.failure(IllegalStateException("OIDC refresh response missing access_token")),
                )
                return@performTokenRequest
            }
            Auth.signInWithOidc(
                ctx = ctx,
                accessToken = accessToken,
                // A refresh may or may not rotate the refresh token; fall
                // back to the prior one when the AS doesn't return a new one.
                refreshToken = tokenResp.refreshToken ?: Auth.refreshToken(ctx),
                accessExpiresAtMs = tokenResp.accessTokenExpirationTime,
                authStateJson = state.jsonSerializeString(),
            )
            onComplete(Result.success(Unit))
        }
    }

    /**
     * Best-effort RP-initiated logout (OpenID Connect Session Management /
     * RP-Initiated Logout) against the storage AS.
     *
     * Discovers the AS metadata for [storageUrl] and, if it advertises an
     * `end_session_endpoint`, opens it in a Custom Tab so the AS can clear
     * its own session cookie. This is **best-effort**: clearing the local
     * session ([Auth.clearSelfSessionToken]) is the must-have and is the
     * caller's responsibility; this call only nudges the AS. Discovery
     * failure / a missing endpoint / a Custom-Tab launch failure are all
     * swallowed — the caller does not block sign-out on the network.
     *
     * AppAuth has no first-class end-session helper across all versions, so
     * we build the URL by hand: `<end_session_endpoint>` is opened directly.
     * Most ASes accept a bare GET; the optional `id_token_hint` /
     * `post_logout_redirect_uri` params are skipped because the app does not
     * retain a parsed id_token and registers no logout redirect.
     */
    fun signOut(ctx: Context, storageUrl: String) {
        val issuerUri = runCatching { Uri.parse(storageUrl.trim()) }.getOrNull() ?: return
        runCatching {
            AuthorizationServiceConfiguration.fetchFromIssuer(
                issuerUri,
            ) { serviceConfig: AuthorizationServiceConfiguration?, ex: AuthorizationException? ->
                if (ex != null || serviceConfig == null) {
                    Log.w(TAG, "sign-out discovery failed; local session already cleared", ex)
                    return@fetchFromIssuer
                }
                val endSession = serviceConfig.discoveryDoc?.endSessionEndpoint
                if (endSession == null) {
                    Log.i(TAG, "AS exposes no end_session_endpoint — local logout only")
                    return@fetchFromIssuer
                }
                runCatching {
                    val intent = Intent(Intent.ACTION_VIEW, endSession).apply {
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                    ctx.startActivity(intent)
                }.onFailure {
                    Log.w(TAG, "could not open end_session_endpoint; local logout stands", it)
                }
            }
        }.onFailure {
            Log.w(TAG, "RP-initiated logout failed; local logout stands", it)
        }
    }

    /**
     * Helper for callers that want a typed
     * [ActivityResultContracts.StartActivityForResult] launcher.
     */
    fun registerForAuthResult(
        activity: ComponentActivity,
        onComplete: (Result<Unit>) -> Unit,
    ): ActivityResultLauncher<Intent> {
        return activity.registerForActivityResult(
            ActivityResultContracts.StartActivityForResult(),
        ) { result ->
            handleAuthResult(activity, result.data, onComplete)
        }
    }

    /** Tear down the cached [AuthorizationService]. */
    fun dispose() {
        synchronized(this) {
            authService?.dispose()
            authService = null
        }
    }
}
