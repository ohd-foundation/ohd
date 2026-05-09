package com.ohd.emergency.ui.screens

import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp

import com.ohd.emergency.BuildConfig
import com.ohd.emergency.data.OidcManager
import com.ohd.emergency.data.OperatorSession

/**
 * Shift-in login screen.
 *
 * Per `SPEC.md` "Auth model on the tablet":
 *
 *     Operator OIDC for the responder (paramedic) at shift-in. Standard
 *     OAuth2 / OIDC against the operator's IdP. Token lives in Android
 *     EncryptedSharedPreferences / iOS Keychain.
 *
 * Two paths:
 *
 *  - **Sign in with operator IdP** — kicks off [OidcManager.startAuthFlow]
 *    which fetches the AS metadata, builds an OAuth Code + PKCE
 *    [AuthorizationRequest], and launches the IdP's Custom Tab. On
 *    success the bearer + refresh + display claims persist via
 *    [OperatorSession.signInWithOidc] (backed by EncryptedSharedPreferences).
 *
 *  - **Dev stub** — keeps the v0 form below. A paramedic on a dev tablet
 *    with no IdP can still smoke-test the rest of the flow.
 *
 * The IdP issuer / client_id come from `BuildConfig` defaults set by the
 * Gradle `manifestPlaceholders` block; the user can override them in
 * the form below for a one-off shift on a guest tablet.
 */
@Composable
fun LoginScreen(onSignedIn: () -> Unit) {
    val ctx = LocalContext.current
    val activity = ctx as? ComponentActivity

    var issuer by remember { mutableStateOf(BuildConfig.OHD_EMERGENCY_OIDC_ISSUER) }
    var clientId by remember { mutableStateOf(BuildConfig.OHD_EMERGENCY_OIDC_CLIENT_ID) }
    var redirectUri by remember { mutableStateOf(BuildConfig.OHD_EMERGENCY_OIDC_REDIRECT) }

    var operatorLabel by remember { mutableStateOf("EMS Prague Region — Crew 42") }
    var responderLabel by remember { mutableStateOf("Officer Novák") }
    var responderSubject by remember { mutableStateOf("nv-2107") }
    var error by remember { mutableStateOf<String?>(null) }
    var pending by remember { mutableStateOf(false) }

    val authLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        OidcManager.handleAuthResult(ctx, result.data) { outcome ->
            pending = false
            outcome
                .onSuccess { onSignedIn() }
                .onFailure { error = it.message ?: "OIDC sign-in failed" }
        }
    }

    Surface(modifier = Modifier.fillMaxSize()) {
        Box(
            modifier = Modifier.fillMaxSize().padding(32.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                modifier = Modifier.widthIn(max = 520.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
                horizontalAlignment = Alignment.Start,
            ) {
                Text(
                    text = "OHD Emergency",
                    style = MaterialTheme.typography.displaySmall,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = "Shift-in. Sign in to your operator's identity provider.",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                Spacer(Modifier.height(8.dp))

                OutlinedTextField(
                    value = issuer,
                    onValueChange = { issuer = it },
                    label = { Text("Issuer URL") },
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

                error?.let { msg ->
                    Text(
                        text = msg,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                    )
                }

                Button(
                    onClick = {
                        if (pending || activity == null) return@Button
                        if (issuer.isBlank() || clientId.isBlank() || redirectUri.isBlank()) {
                            error = "Issuer / client / redirect are required."
                            return@Button
                        }
                        error = null
                        pending = true
                        OidcManager.startAuthFlow(
                            activity = activity,
                            launcher = authLauncher,
                            config = OidcManager.Config(
                                issuer = issuer.trim(),
                                clientId = clientId.trim(),
                                redirectUri = redirectUri.trim(),
                            ),
                            onError = { msg ->
                                pending = false
                                error = msg
                            },
                        )
                    },
                    enabled = !pending,
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.primary,
                        contentColor = MaterialTheme.colorScheme.onPrimary,
                    ),
                ) {
                    Text(
                        if (pending) "Opening browser…" else "Sign in with operator IdP",
                        style = MaterialTheme.typography.titleMedium,
                    )
                }

                Spacer(Modifier.height(8.dp))
                HorizontalDivider()
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "Dev stub (no IdP available)",
                    style = MaterialTheme.typography.titleSmall,
                )

                OutlinedTextField(
                    value = operatorLabel,
                    onValueChange = { operatorLabel = it },
                    label = { Text("Operator (org + crew)") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = responderLabel,
                    onValueChange = { responderLabel = it },
                    label = { Text("Responder (your name)") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = responderSubject,
                    onValueChange = { responderSubject = it },
                    label = { Text("Responder ID (operator IdP subject)") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                    modifier = Modifier.fillMaxWidth(),
                )

                OutlinedButton(
                    onClick = {
                        if (operatorLabel.isBlank() || responderLabel.isBlank() || responderSubject.isBlank()) {
                            error = "All three stub fields are required."
                            return@OutlinedButton
                        }
                        OperatorSession.stubSignIn(
                            ctx = ctx,
                            operatorLabel = operatorLabel.trim(),
                            responderLabel = responderLabel.trim(),
                            responderSubject = responderSubject.trim(),
                        )
                        onSignedIn()
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Sign in (stub)")
                }
            }
        }
    }
}
