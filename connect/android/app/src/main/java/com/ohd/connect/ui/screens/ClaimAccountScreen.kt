package com.ohd.connect.ui.screens

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.OhdSaasClient
import com.ohd.connect.data.OhdSaasTokenStore
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.launch

/**
 * "Already have an account?" — takes a 16×8 recovery code, calls
 * `/v1/account/recover`, persists the returned access token, and
 * forwards [onClaimed] with the matched `profile_ulid`. The caller is
 * expected to overwrite the local [com.ohd.connect.data.OhdAccountStore]
 * with the recovered profile before forwarding the user into the app.
 */
@Composable
fun ClaimAccountScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onClaimed: (profileUlid: String) -> Unit,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        OhdTopBar(title = "Connect existing account", onBack = onBack)

        Spacer(Modifier.height(8.dp))
        Text(
            text = "Paste the recovery code you saved earlier (16 lines × 8 characters). Spaces, dashes and case are ignored.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            lineHeight = 19.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
        )

        Spacer(Modifier.height(12.dp))
        Column(modifier = Modifier.padding(horizontal = 16.dp)) {
            OhdField(
                label = "Recovery code",
                value = code,
                onValueChange = { code = it },
                placeholder = "ABCD EFGH IJKL MNOP …",
                keyboardType = KeyboardType.Ascii,
            )
        }

        if (error != null) {
            Spacer(Modifier.height(8.dp))
            Text(
                text = error!!,
                color = OhdColors.Red,
                fontFamily = OhdBody,
                fontSize = 13.sp,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }

        Spacer(Modifier.height(16.dp))
        OhdButton(
            label = if (busy) "Checking…" else "Recover account",
            onClick = {
                if (busy) return@OhdButton
                error = null
                busy = true
                scope.launch {
                    OhdSaasClient.recover(code).fold(
                        onSuccess = { res ->
                            OhdSaasTokenStore.save(ctx, res.accessToken)
                            // The recovered profile may differ from any
                            // locally-minted one. Caller decides whether to
                            // overwrite the local store.
                            onClaimed(res.profileUlid)
                        },
                        onFailure = {
                            error = "Recovery failed — check the code or try again later."
                            Toast.makeText(ctx, error, Toast.LENGTH_SHORT).show()
                        },
                    )
                    busy = false
                }
            },
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        )

        Spacer(Modifier.height(8.dp))
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "Recovery requires api.ohd.dev to be reachable. If the service is unavailable, keep using local storage and try again later.",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                lineHeight = 18.sp,
                color = OhdColors.Muted,
            )
        }
        Spacer(Modifier.height(24.dp))
    }
}
