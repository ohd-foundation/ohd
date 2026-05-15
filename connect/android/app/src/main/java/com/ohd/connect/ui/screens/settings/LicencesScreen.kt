package com.ohd.connect.ui.screens.settings

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.OssLicences
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Settings → About → Open-source licences.
 *
 * One LazyColumn rendering each [OssLicences.OssLib] under its category
 * header. Tapping a row opens the upstream URL in the system browser.
 *
 * The list intentionally exposes `groupArtifact · version · spdx` on the
 * secondary line so a curious user (or a clinician trying to audit the
 * build) can grep / paste into their compliance system without diving
 * into the URL.
 */
@Composable
fun LicencesScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
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

    // Group entries up-front so the LazyColumn body is a flat list of rows
    // interspersed with section headers — easier to reason about and
    // cheaper than `groupBy` on every recomposition.
    val grouped = remember { OssLicences.byCategory }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Open-source licences", onBack = onBack)

        LazyColumn(modifier = Modifier.fillMaxSize()) {
            item("hint") {
                Text(
                    text = "Tap a library to open its upstream page.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    lineHeight = 18.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }

            for ((category, libs) in grouped) {
                item("header-${category.name}") {
                    OhdSectionHeader(category.display)
                }
                items(libs, key = { "${it.groupArtifact}-${it.version}" }) { lib ->
                    OhdListItem(
                        primary = lib.name,
                        secondary = listOfNotNull(
                            lib.groupArtifact,
                            lib.version,
                            lib.licence.spdx,
                        ).joinToString(" · "),
                        meta = "›",
                        onClick = { openUrl(lib.url) },
                    )
                    if (lib.note != null) {
                        Text(
                            text = lib.note,
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W400,
                            fontSize = 11.sp,
                            lineHeight = 16.sp,
                            color = OhdColors.Muted,
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(start = 16.dp, end = 16.dp, bottom = 6.dp),
                        )
                    }
                    OhdDivider()
                }
                item("spacer-${category.name}") {
                    Spacer(Modifier.height(8.dp))
                }
            }

            item("footer-spacer") {
                Spacer(Modifier.height(24.dp))
            }
        }
    }
}
