package com.ohd.connect.ui.screens.settings

import android.content.Intent
import android.net.Uri
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.BuildConfig
import com.ohd.connect.data.OssLicences
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Settings → About — identity card, repo / spec quick links, and the entry
 * point into the open-source licences list.
 *
 * Version comes from `BuildConfig.VERSION_NAME` (set in `app/build.gradle.kts`);
 * the variant ("debug" / "release") follows `BuildConfig.BUILD_TYPE`.
 * Surfaces a single tappable row that pushes [onOpenLicences]; the row text includes
 * the live count from [OssLicences.count] so it stays in sync if new deps
 * are appended to the registry.
 */
@Composable
fun AboutScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onOpenLicences: () -> Unit,
) {
    val ctx = LocalContext.current

    val openUrl: (String) -> Unit = { url ->
        runCatching {
            ctx.startActivity(
                Intent(Intent.ACTION_VIEW, Uri.parse(url))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "About", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
        ) {
            // --- Identity card ---
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 16.dp),
            ) {
                OhdCard {
                    Text(
                        text = "OHD",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W600,
                        fontSize = 22.sp,
                        color = OhdColors.Ink,
                    )
                    Text(
                        text = "Connect for Android",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        color = OhdColors.Muted,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "version ${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD_TYPE})",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 12.sp,
                        color = OhdColors.Muted,
                    )
                }
            }

            // --- Resources section ---
            OhdSectionHeader("Resources")

            OhdListItem(
                primary = "Open-source licences",
                secondary = "${OssLicences.count} libraries",
                meta = "›",
                onClick = onOpenLicences,
            )
            OhdDivider()

            OhdListItem(
                primary = "Repository",
                secondary = "github.com/ohd-foundation/ohd",
                meta = "↗",
                onClick = { openUrl("https://github.com/ohd-foundation/ohd") },
            )
            OhdDivider()

            OhdListItem(
                primary = "Specification",
                secondary = "github.com/ohd-foundation/ohd/tree/main/spec",
                meta = "↗",
                onClick = { openUrl("https://github.com/ohd-foundation/ohd/tree/main/spec") },
            )

            Spacer(Modifier.height(24.dp))

            // --- Footer note ---
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.Center,
            ) {
                Text(
                    text = "OHD Connect is licensed under Apache-2.0 OR MIT. Pick whichever you prefer.",
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
}
