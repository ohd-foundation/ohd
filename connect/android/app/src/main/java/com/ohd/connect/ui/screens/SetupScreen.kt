package com.ohd.connect.ui.screens

import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.ohd.connect.BuildConfig
import com.ohd.connect.data.Auth
import com.ohd.connect.data.OidcManager
import com.ohd.connect.data.StorageRepository

/**
 * First-run setup screen.
 *
 * Three paths:
 *  - **Use on-device storage** — calls `OhdStorage.create(...)`. The Rust
 *    core stamps `_meta.user_ulid`, runs migrations, the app then mints a
 *    self-session token and writes it into [Auth].
 *  - **Connect to remote storage** — kicks off [OidcManager.startAuthFlow]
 *    against the storage URL the user pastes. This runs OAuth Code + PKCE
 *    in a Custom Tab via AppAuth-Android; the resulting `ohds_…` access
 *    token persists via [Auth.signInWithOidc] (backed by
 *    `EncryptedSharedPreferences`).
 *  - **(Stub) skip auth** — kept off the screen; only surfaces in dev builds.
 *
 * The v0 on-device path collapses key derivation to a stub passphrase
 * (see `StorageRepository.openOrCreate` for the upgrade path).
 */
@Composable
fun SetupScreen(onSetupDone: () -> Unit) {
    val ctx = LocalContext.current
    val activity = ctx as? ComponentActivity
    var status by remember { mutableStateOf<String?>(null) }
    var inFlight by remember { mutableStateOf(false) }

    // Remote-storage form (visible after the user picks "Connect to a remote
    // storage"). Pre-fills from BuildConfig defaults so a dev tablet with
    // -Pohd.connect.oidc.storage_url already set doesn't have to retype.
    var showRemoteForm by remember { mutableStateOf(false) }
    var storageUrl by remember { mutableStateOf(BuildConfig.OHD_OIDC_STORAGE_URL) }
    var clientId by remember { mutableStateOf(BuildConfig.OHD_OIDC_CLIENT_ID) }
    var redirectUri by remember { mutableStateOf(BuildConfig.OHD_OIDC_REDIRECT) }

    val authLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        OidcManager.handleAuthResult(ctx, result.data) { outcome ->
            inFlight = false
            outcome
                .onSuccess {
                    status = "Signed in to remote storage."
                    onSetupDone()
                }
                .onFailure { status = "Sign-in failed: ${it.message}" }
        }
    }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 24.dp, vertical = 32.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Column {
                Text(
                    text = "OHD Connect",
                    style = MaterialTheme.typography.headlineLarge,
                )
                Spacer(Modifier.height(12.dp))
                Text(
                    text = "Your health data, on your terms.",
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(24.dp))
                Text(
                    text = "Pick where your data lives. You can change this later in Settings.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Button(
                    onClick = {
                        if (inFlight) return@Button
                        inFlight = true
                        status = "Creating storage…"

                        // TODO: real key derivation per spec/encryption.md.
                        //       For v0 we use a deterministic stub key so
                        //       Stage 1's SQLCipher PRAGMA key is well-formed.
                        val stubKeyHex = "00".repeat(32)

                        val openResult = StorageRepository.openOrCreate(stubKeyHex)
                        openResult
                            .onFailure { e ->
                                status = "Storage open failed: ${e.message}"
                                inFlight = false
                            }
                            .onSuccess {
                                StorageRepository.issueSelfSessionToken()
                                    .onFailure { e2 ->
                                        status = "Token issue failed: ${e2.message}"
                                        inFlight = false
                                    }
                                    .onSuccess {
                                        Auth.markFirstRunDone(ctx)
                                        inFlight = false
                                        onSetupDone()
                                    }
                            }
                    },
                    enabled = !inFlight,
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.primary,
                        contentColor = MaterialTheme.colorScheme.onPrimary,
                    ),
                ) {
                    Text("Use on-device storage")
                }
                OutlinedButton(
                    onClick = {
                        showRemoteForm = !showRemoteForm
                        if (!showRemoteForm) status = null
                    },
                    enabled = !inFlight,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        if (showRemoteForm) "Hide remote storage form" else "Connect to a remote storage",
                    )
                }

                if (showRemoteForm) {
                    Spacer(Modifier.height(8.dp))
                    HorizontalDivider()
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = storageUrl,
                        onValueChange = { storageUrl = it },
                        label = { Text("Storage URL") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = clientId,
                        onValueChange = { clientId = it },
                        label = { Text("Client ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = redirectUri,
                        onValueChange = { redirectUri = it },
                        label = { Text("Redirect URI") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(
                        onClick = {
                            if (inFlight || activity == null) return@Button
                            if (storageUrl.isBlank() || clientId.isBlank() || redirectUri.isBlank()) {
                                status = "Storage URL / client / redirect are required."
                                return@Button
                            }
                            status = "Opening browser for sign-in…"
                            inFlight = true
                            OidcManager.startAuthFlow(
                                activity = activity,
                                launcher = authLauncher,
                                config = OidcManager.Config(
                                    storageUrl = storageUrl.trim(),
                                    clientId = clientId.trim(),
                                    redirectUri = redirectUri.trim(),
                                ),
                                onError = { msg ->
                                    inFlight = false
                                    status = "Sign-in failed: $msg"
                                },
                            )
                        },
                        enabled = !inFlight,
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.primary,
                            contentColor = MaterialTheme.colorScheme.onPrimary,
                        ),
                    ) {
                        Text("Sign in to remote storage")
                    }
                }

                status?.let { msg ->
                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = msg,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
