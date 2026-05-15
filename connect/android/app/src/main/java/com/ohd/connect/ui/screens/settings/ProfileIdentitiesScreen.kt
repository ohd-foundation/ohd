package com.ohd.connect.ui.screens.settings

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.LinkedIdentity
import com.ohd.connect.data.OhdAccountStore
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/**
 * Settings → Profile & Access → Linked identities.
 *
 * Shows the OIDC providers the user has linked to their profile. Each row
 * displays the provider issuer + opaque `sub` (the storage-server token
 * we'd send) plus an "Unlink" affordance. "Add" buttons launch the OIDC
 * flow when `api.ohd.dev` is reachable; for now they surface a toast
 * pointing at the roadmap.
 */
@Composable
fun ProfileIdentitiesScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current
    var identities by remember { mutableStateOf<List<LinkedIdentity>>(emptyList()) }
    LaunchedEffect(Unit) {
        identities = OhdAccountStore.load(ctx)?.linkedIdentities.orEmpty()
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        OhdTopBar(title = "Linked identities", onBack = onBack)

        Spacer(Modifier.height(8.dp))
        Text(
            text = "Link an OIDC provider for faster account recovery. The recovery code stays as the fallback — providers are convenience, not a replacement.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            lineHeight = 19.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        )

        if (identities.isEmpty()) {
            Spacer(Modifier.height(12.dp))
            OhdCard {
                Text(
                    text = "No identities linked yet.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
            }
        } else {
            OhdSectionHeader("LINKED")
            identities.forEach { row ->
                OhdCard {
                    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            text = row.displayLabel ?: row.provider,
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W500,
                            fontSize = 14.sp,
                            color = OhdColors.Ink,
                        )
                        Text(
                            text = "${row.provider} · sub=${row.sub.take(12)}…",
                            fontFamily = OhdMono,
                            fontSize = 11.sp,
                            color = OhdColors.Muted,
                        )
                        Spacer(Modifier.height(6.dp))
                        Row(horizontalArrangement = Arrangement.End, modifier = Modifier.fillMaxWidth()) {
                            OhdButton(
                                label = "Unlink",
                                variant = OhdButtonVariant.Ghost,
                                onClick = {
                                    OhdAccountStore.removeLinkedIdentity(ctx, row.provider, row.sub)
                                    identities = OhdAccountStore.load(ctx)?.linkedIdentities.orEmpty()
                                },
                            )
                        }
                    }
                }
            }
        }

        Spacer(Modifier.height(16.dp))
        OhdSectionHeader("ADD")
        OhdCard {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = "Identity linking goes live once api.ohd.dev ships. Until then your recovery code is the only path.",
                    fontFamily = OhdBody,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    AddProviderButton("Google", ctx)
                    AddProviderButton("Apple", ctx)
                    AddProviderButton("Email", ctx)
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun AddProviderButton(label: String, ctx: android.content.Context) {
    OhdButton(
        label = label,
        variant = OhdButtonVariant.Ghost,
        onClick = {
            Toast.makeText(ctx, "$label OIDC linking coming soon", Toast.LENGTH_SHORT).show()
        },
    )
}
