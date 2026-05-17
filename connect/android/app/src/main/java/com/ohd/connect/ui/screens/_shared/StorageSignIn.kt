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
 * Holder bundling the registered Custom-Tab launcher with the option +
 * URL of the sign-in currently in flight. [launch] kicks off discovery +
 * the Custom Tab; the result lands in the `onResult` passed to
 * [rememberStorageAuthLauncher].
 */
class StorageAuthLauncher internal constructor(
    private val activity: ComponentActivity,
    private val launcher: ActivityResultLauncher<Intent>,
    private val onError: (String) -> Unit,
) {
    /** The (option, url) pair whose sign-in is currently being processed. */
    internal var pending: Pair<StorageOption, String>? = null
        private set

    /**
     * Begin the OIDC Code + PKCE flow for [option] against [storageUrl].
     * Opens the storage AS in a Custom Tab. The success/failure callback
     * registered with [rememberStorageAuthLauncher] fires on return.
     */
    fun launch(option: StorageOption, storageUrl: String) {
        pending = option to storageUrl
        OidcManager.startAuthFlow(
            activity = activity,
            launcher = launcher,
            config = OidcManager.Config(
                storageUrl = storageUrl.trim(),
                clientId = BuildConfig.OHD_OIDC_CLIENT_ID,
                redirectUri = BuildConfig.OHD_OIDC_REDIRECT,
            ),
            onError = { msg ->
                pending = null
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
    // Mutable holder so the result callback can read the in-flight pair
    // after the launcher itself is constructed.
    val holderRef = androidx.compose.runtime.remember {
        arrayOfNulls<StorageAuthLauncher>(1)
    }
    val ctx = activity
    val launcher = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        val holder = holderRef[0]
        val pending = holder?.pending
        OidcManager.handleAuthResult(ctx, result.data) { outcome ->
            outcome
                .onSuccess {
                    if (pending != null) {
                        val (option, url) = pending
                        Auth.saveStorageUrl(ctx, option.name, url)
                        Auth.saveStorageOption(ctx, option.name)
                        onResult(StorageSignInResult.Success(option, url))
                    } else {
                        onResult(
                            StorageSignInResult.Failure(
                                "Signed in, but the storage option was lost — please retry.",
                            ),
                        )
                    }
                }
                .onFailure {
                    onResult(
                        StorageSignInResult.Failure(it.message ?: "Sign-in failed"),
                    )
                }
        }
    }
    val holder = androidx.compose.runtime.remember(launcher) {
        StorageAuthLauncher(
            activity = activity,
            launcher = launcher,
            onError = { msg -> onResult(StorageSignInResult.Failure(msg)) },
        ).also { holderRef[0] = it }
    }
    return holder
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
