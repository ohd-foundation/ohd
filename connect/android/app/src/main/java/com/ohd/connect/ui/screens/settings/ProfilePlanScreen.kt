package com.ohd.connect.ui.screens.settings

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
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
import com.ohd.connect.data.OhdAccountStore
import com.ohd.connect.data.Plan
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/**
 * Settings → Profile & Access → Plan.
 *
 * Surfaces the current tier (Free / Paid) and the retention limit it
 * implies, plus an Upgrade CTA that bounces to the OHD SaaS checkout
 * (stubbed to the roadmap page today; real Stripe flow lands when
 * `api.ohd.dev/v1/account/plan/checkout` ships).
 */
@Composable
fun ProfilePlanScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current
    var plan by remember { mutableStateOf<Plan?>(null) }
    LaunchedEffect(Unit) { plan = OhdAccountStore.load(ctx)?.plan ?: Plan.Free }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        OhdTopBar(title = "Plan", onBack = onBack)

        OhdSectionHeader("CURRENT")
        OhdCard {
            Column {
                Text(
                    text = when (plan) {
                        Plan.Paid -> "OHD Cloud — Paid"
                        else -> "Free"
                    },
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W600,
                    fontSize = 18.sp,
                    color = OhdColors.Ink,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    text = when (plan) {
                        Plan.Paid ->
                            "Unlimited retention · 5 GB storage · sync across devices · recovery delegation."
                        else ->
                            "7-day rolling retention · 25 MB storage · local only. Events older than a week are deleted automatically."
                    },
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    lineHeight = 18.sp,
                    color = OhdColors.Muted,
                )
            }
        }

        if (plan != Plan.Paid) {
            Spacer(Modifier.height(16.dp))
            OhdSectionHeader("UPGRADE")
            OhdCard {
                Column {
                    Text(
                        text = "OHD Cloud unlocks unlimited retention, sync across devices, and account recovery via OHD's relays.",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 13.sp,
                        lineHeight = 18.sp,
                        color = OhdColors.Muted,
                    )
                    Spacer(Modifier.height(12.dp))
                    OhdButton(
                        label = "Open checkout",
                        variant = OhdButtonVariant.Primary,
                        onClick = {
                            val intent = Intent(
                                Intent.ACTION_VIEW,
                                Uri.parse("https://ohd.dev/roadmap.html#payments"),
                            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                            runCatching { ctx.startActivity(intent) }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }

        Spacer(Modifier.height(16.dp))
        OhdSectionHeader("DETAILS")
        OhdCard {
            Column {
                PlanDetailRow("Retention", if (plan == Plan.Paid) "Unlimited" else "7 days")
                PlanDetailRow("Storage cap", if (plan == Plan.Paid) "5 GB" else "25 MB")
                PlanDetailRow("Cross-device sync", if (plan == Plan.Paid) "Enabled" else "Free tier only")
                PlanDetailRow("Recovery delegation", if (plan == Plan.Paid) "OHD relays" else "Recovery code only")
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun PlanDetailRow(label: String, value: String) {
    androidx.compose.foundation.layout.Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        horizontalArrangement = androidx.compose.foundation.layout.Arrangement.SpaceBetween,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
        )
        Text(
            text = value,
            fontFamily = OhdMono,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
    }
}

@Suppress("UNUSED_PARAMETER")
private fun roundedCornerShape() = RoundedCornerShape(8.dp)
