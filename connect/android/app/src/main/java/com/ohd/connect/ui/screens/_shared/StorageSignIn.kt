package com.ohd.connect.ui.screens._shared

import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.input.KeyboardType
import com.ohd.connect.BuildConfig
import com.ohd.connect.data.Auth
import com.ohd.connect.data.OidcManager
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Phase 2 — storage picker → OIDC login glue.
 *
 * `OidcManager` (Code + PKCE in a Custom Tab) was dead code: no UI screen
 * launched it. This file is the shared bridge wired into both storage
 * pickers — the in-Settings `StorageSettingsScreen` and the first-run
 * `OnboardingStorageScreen`.
 *
 * The Custom Tab is launched via an `ActivityResultLauncher<Intent>`
 * obtained from [OidcManager.registerForAuthResult] — that helper both
 * registers the launcher AND wires the redirect back through
 * [OidcManager.handleAuthResult] (which persists the `ohds_`/`ohdr_`
 * tokens via [Auth.signInWithOidc]). Registration must happen during
 * composition before `STARTED`, so picker screens call
 * [rememberStorageAuthLauncher] at the top of their composable.
 *
 * Phase 2 stops at "tokens + URL + option persisted". The data path still
 * opens local storage — Phase 3 reads [Auth.loadStorageUrl] +
 * [Auth.getSelfSessionToken] to construct the remote backend.
 */

/** Outcome of a storage OIDC sign-in attempt, surfaced to the picker. */
sealed interface StorageSignInResult {
    data class Success(val option: StorageOption, val storageUrl: String) : StorageSignInResult
    data class Failure(val message: String) : StorageSignInResult
}

/**
 * Holder bundling the registered Custom-Tab launcher. [launch] kicks off
 * discovery + the Custom Tab; the result lands in the `onResult` passed to
 * [rememberStorageAuthLauncher].
 *
 * The in-flight (option, url) is persisted via [Auth.savePendingStorageSignIn]
 * rather than held in memory: the Custom Tab backgrounds the app for the
 * whole login, long enough for the OS to kill the activity/process, so the
 * redirect must be attributable from disk after a cold recreation.
 */
class StorageAuthLauncher internal constructor(
    private val activity: ComponentActivity,
    private val launcher: ActivityResultLauncher<Intent>,
    private val onError: (String) -> Unit,
) {
    /**
     * Begin the OIDC Code + PKCE flow for [option] against [storageUrl].
     * Opens the storage AS in a Custom Tab. The success/failure callback
     * registered with [rememberStorageAuthLauncher] fires on return.
     */
    fun launch(option: StorageOption, storageUrl: String) {
        Auth.savePendingStorageSignIn(activity, option.name, storageUrl)
        OidcManager.startAuthFlow(
            activity = activity,
            launcher = launcher,
            config = OidcManager.Config(
                storageUrl = storageUrl.trim(),
                clientId = BuildConfig.OHD_OIDC_CLIENT_ID,
                redirectUri = BuildConfig.OHD_OIDC_REDIRECT,
                // OHD Cloud → tell the storage AS the provider up front so it
                // skips its picker page and redirects straight to sign-in.
                additionalParams = if (option == StorageOption.OhdCloud) {
                    mapOf("provider" to "ohd_account")
                } else {
                    emptyMap()
                },
            ),
            onError = { msg ->
                Auth.clearPendingStorageSignIn(activity)
                onError(msg)
            },
        )
    }
}

/**
 * Register the Custom-Tab `ActivityResultLauncher` for a storage picker
 * screen and return a [StorageAuthLauncher] the picker uses to start the
 * flow.
 *
 * On a successful redirect [OidcManager.handleAuthResult] has already
 * persisted the `ohds_`/`ohdr_` tokens via [Auth.signInWithOidc]; this
 * helper additionally persists the storage URL ([Auth.saveStorageUrl]) and
 * the selected option ([Auth.saveStorageOption]) before invoking [onResult].
 */
@Composable
fun rememberStorageAuthLauncher(
    activity: ComponentActivity,
    onResult: (StorageSignInResult) -> Unit,
): StorageAuthLauncher {
    val ctx = activity
    val launcher = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        OidcManager.handleAuthResult(ctx, result.data) { outcome ->
            // The in-flight option/url is read from disk, not memory — the
            // result may be redelivered to a freshly recreated activity
            // whose Compose state was wiped while the Custom Tab was up.
            val pending = Auth.loadPendingStorageSignIn(ctx)
            outcome
                .onSuccess {
                    val option = pending
                        ?.let { (name, _) ->
                            StorageOption.entries.firstOrNull { it.name == name }
                        }
                    if (pending != null && option != null) {
                        val url = pending.second
                        Auth.saveStorageUrl(ctx, option.name, url)
                        Auth.saveStorageOption(ctx, option.name)
                        Auth.clearPendingStorageSignIn(ctx)
                        onResult(StorageSignInResult.Success(option, url))
                    } else {
                        Auth.clearPendingStorageSignIn(ctx)
                        onResult(
                            StorageSignInResult.Failure(
                                "Signed in, but the storage option was lost — please retry.",
                            ),
                        )
                    }
                }
                .onFailure {
                    Auth.clearPendingStorageSignIn(ctx)
                    onResult(
                        StorageSignInResult.Failure(it.message ?: "Sign-in failed"),
                    )
                }
        }
    }
    return androidx.compose.runtime.remember(launcher) {
        StorageAuthLauncher(
            activity = activity,
            launcher = launcher,
            onError = { msg -> onResult(StorageSignInResult.Failure(msg)) },
        )
    }
}

/** True iff [option] is a remote (non-on-device) storage option. */
fun StorageOption.isRemote(): Boolean = this != StorageOption.OnDevice

/**
 * Whether [option] needs the user to type a storage URL. `OhdCloud` uses a
 * fixed [BuildConfig.OHD_CLOUD_STORAGE_URL]; self/provider-hosted require
 * the user's own server URL.
 */
fun StorageOption.needsUrlField(): Boolean =
    this == StorageOption.SelfHosted || this == StorageOption.ProviderHosted

/** The fixed URL for [StorageOption.OhdCloud], or `null` for other options. */
fun StorageOption.fixedStorageUrl(): String? =
    if (this == StorageOption.OhdCloud) BuildConfig.OHD_CLOUD_STORAGE_URL else null

/**
 * Sign-in panel rendered inside a selected remote storage card.
 *
 *  - `OhdCloud`: shows a "Sign in to OHD Cloud" button against the fixed
 *    cloud URL.
 *  - `SelfHosted` / `ProviderHosted`: shows a URL text field + a sign-in
 *    button enabled once the field is non-blank.
 *
 * Once the user has a persisted token + URL for this option, the panel
 * collapses to a green "Signed in" row instead.
 *
 * [urlValue] / [onUrlChange] hoist the URL field state to the caller so it
 * survives recomposition; [onSignIn] is invoked with the resolved URL.
 */
@Composable
fun StorageSignInPanel(
    option: StorageOption,
    signedIn: Boolean,
    signedInUrl: String?,
    inFlight: Boolean,
    urlValue: String,
    onUrlChange: (String) -> Unit,
    onSignIn: (String) -> Unit,
    errorMessage: String? = null,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (signedIn) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(
                    imageVector = OhdIcons.ShieldCheck,
                    contentDescription = null,
                    tint = OhdColors.Success,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    text = "Signed in" + (signedInUrl?.let { " · $it" } ?: ""),
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 12.sp,
                    color = OhdColors.Success,
                    modifier = Modifier.weight(1f),
                )
            }
            return@Column
        }

        if (option.needsUrlField()) {
            OhdInput(
                value = urlValue,
                onValueChange = onUrlChange,
                placeholder = "https://your-storage.example",
                leadingIcon = OhdIcons.Server,
                keyboardType = KeyboardType.Uri,
            )
        }

        if (errorMessage != null) {
            Text(
                text = errorMessage,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 12.sp,
                color = OhdColors.Red,
            )
        }

        val resolvedUrl = option.fixedStorageUrl() ?: urlValue.trim()
        val canSignIn = !inFlight && resolvedUrl.isNotBlank()
        OhdButton(
            label = when {
                inFlight -> "Opening sign-in…"
                option == StorageOption.OhdCloud -> "Sign in to OHD Cloud"
                else -> "Sign in to this server"
            },
            onClick = { if (canSignIn) onSignIn(resolvedUrl) },
            modifier = Modifier.fillMaxWidth(),
            enabled = canSignIn,
        )
    }
}
