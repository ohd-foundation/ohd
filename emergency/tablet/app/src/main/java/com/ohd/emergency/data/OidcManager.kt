package com.ohd.emergency.data

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
 * Operator-OIDC manager for the paramedic tablet.
 *
 * Mirrors the connect/web + connect/cli OIDC story for a Compose+Kotlin
 * Android app: the responder signs in against the operator's IdP via
 * AppAuth-Android (OAuth 2.0 Authorization Code + PKCE in a Custom Tab),
 * the resulting bearer is persisted via [OperatorSession] (which now
 * uses `EncryptedSharedPreferences`), and the rest of the app reads
 * the bearer from [OperatorSession.bearer].
 *
 * Configuration via Gradle BuildConfig fields (set in `app/build.gradle.kts`)
 * or — fallback — a runtime override on the LoginScreen. The fields
 * default to placeholders so the app builds without a deployment-specific
 * issuer baked in:
 *
 *   OHD_EMERGENCY_OIDC_ISSUER:    https://idp.example.cz/realms/ems
 *   OHD_EMERGENCY_OIDC_CLIENT_ID: ohd-emergency-tablet
 *   OHD_EMERGENCY_OIDC_REDIRECT:  com.ohd.emergency:/oidc-callback
 *
 * The redirect URI **must** match the redirect-scheme intent-filter in
 * `AndroidManifest.xml` (added by the AppAuth-Android `manifestPlaceholders`
 * mechanism in `app/build.gradle.kts`'s `appAuthRedirectScheme`).
 *
 * Threading: AppAuth's [AuthorizationService] is built lazily; every public
 * method is safe to call on the main thread.
 *
 * ### Compose entry-points
 *
 * From a `@Composable`:
 *
 * ```kotlin
 * val activity = LocalContext.current as ComponentActivity
 * val launcher = rememberLauncherForActivityResult(...) { resultIntent ->
 *     OidcManager.handleAuthResult(activity, resultIntent) { result ->
 *         // navigate to /discovery on success
 *     }
 * }
 * Button(onClick = { OidcManager.startAuthFlow(activity, launcher, config) })
 * ```
 *
 * Or, for callers that want to manage the launcher themselves, [authIntent]
 * returns the raw Intent to launch.
 */
object OidcManager {

    private const val TAG = "OhdEmergency.OIDC"

    /** Per-config snapshot consumed by [startAuthFlow]. */
    data class Config(
        val issuer: String,
        val clientId: String,
        val redirectUri: String,
        val scope: String = "openid profile offline_access",
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
     * Discover the AS metadata for [Config.issuer] and return an Intent
     * that, when launched via [ActivityResultLauncher], pops the IdP's
     * Custom Tab. The Intent encodes the PKCE verifier + state; AppAuth
     * persists those in its own state file across the round-trip.
     */
    fun authIntent(
        ctx: Context,
        config: Config,
        onError: (String) -> Unit,
        onIntent: (Intent) -> Unit,
    ) {
        val issuerUri = Uri.parse(config.issuer)
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
                .build()
            onIntent(service(ctx).getAuthorizationRequestIntent(req))
        }
    }

    /**
     * Convenience: kick off the flow on a [ComponentActivity] using the
     * pre-registered [launcher]. Mirrors the typical AppAuth example.
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
     * On success, persists the access / refresh tokens + display claims
     * to [OperatorSession] (which uses `EncryptedSharedPreferences`).
     * On failure, surfaces an error string for the LoginScreen to render.
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
            val claims = tokenResp.idToken?.let { decodeJwtClaims(it) } ?: emptyMap()
            val operatorLabel = (claims["org_name"] as? String)
                ?: (claims["operator_name"] as? String)
                ?: (claims["aud"] as? String)
                ?: "operator"
            val responderLabel = (claims["name"] as? String)
                ?: (claims["preferred_username"] as? String)
                ?: "responder"
            val responderSubject = (claims["sub"] as? String) ?: ""

            OperatorSession.signInWithOidc(
                ctx = ctx,
                bearer = accessToken,
                refreshToken = tokenResp.refreshToken,
                accessExpiresAtMs = tokenResp.accessTokenExpirationTime,
                operatorLabel = operatorLabel,
                responderLabel = responderLabel,
                responderSubject = responderSubject,
                authStateJson = state.jsonSerializeString(),
            )
            onComplete(Result.success(Unit))
        }
    }

    /**
     * Helper for callers that want a typed [ActivityResultContracts.StartActivityForResult]
     * launcher tied to [handleAuthResult].
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

    /**
     * Tear down the cached [AuthorizationService]. Call from
     * `Activity.onDestroy` if the launcher was registered there;
     * otherwise the application context lifetime is fine.
     */
    fun dispose() {
        synchronized(this) {
            authService?.dispose()
            authService = null
        }
    }

    /**
     * Best-effort base64url-decode of the JWT body. **Not** a verified parse.
     * The id_token is consumed only to surface display fields; the access
     * token is what's actually checked server-side by storage / relay.
     */
    private fun decodeJwtClaims(jwt: String): Map<String, Any?> {
        val parts = jwt.split(".")
        if (parts.size < 2) return emptyMap()
        val padded = parts[1]
            .replace('-', '+')
            .replace('_', '/')
        // Pad to multiple of 4.
        val pad = (4 - padded.length % 4) % 4
        val decoded = android.util.Base64.decode(
            padded + "=".repeat(pad),
            android.util.Base64.DEFAULT,
        )
        return runCatching {
            val text = String(decoded, Charsets.UTF_8)
            parseFlatJsonObject(text)
        }.getOrDefault(emptyMap())
    }

    /**
     * Tiny zero-dep flat JSON parser. Only handles the subset we care
     * about for id_token claims (string / number / null at the top level).
     * Avoids pulling in Moshi / kotlinx.serialization just for two
     * claim reads. Returns an empty map if the input is malformed.
     */
    private fun parseFlatJsonObject(text: String): Map<String, Any?> {
        val out = mutableMapOf<String, Any?>()
        val obj = org.json.JSONObject(text)
        val keys = obj.keys()
        while (keys.hasNext()) {
            val k = keys.next()
            out[k] = when (val v = obj.opt(k)) {
                org.json.JSONObject.NULL -> null
                else -> v
            }
        }
        return out
    }
}
