package com.ohd.connect.ui.screens

import android.os.Build
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.HealthConnectPrefs
import com.ohd.connect.data.OhdHealthConnect
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Sources screen — drill-down from the Home "1 source" stat tile.
 *
 * Inventory of everything that produces events into this OHD store:
 * the phone itself, connected sources like Health Connect, paired wearables,
 * etc. Bottom row offers two distinct actions: **Add source** (live pairing
 * — Health Connect grants, BLE, OIDC sessions) and **Import data** (one-shot
 * file imports — Samsung ECG CSV today, generic CSV / JSONL behind it).
 */
@Composable
fun SourcesScreen(
    onBack: () -> Unit,
    onOpenHealthConnect: () -> Unit,
    onImportData: () -> Unit,
    contentPadding: PaddingValues,
) {
    val ctx = LocalContext.current

    var eventCount by remember { mutableLongStateOf(0L) }
    LaunchedEffect(Unit) {
        eventCount = withContext(Dispatchers.IO) {
            StorageRepository.countEvents(EventFilter()).getOrNull() ?: 0L
        }
    }

    val hcAvailability = remember { OhdHealthConnect.availability(ctx) }
    var hcGrantedCount by remember { mutableStateOf(0) }
    val hcLastSyncMs = remember { HealthConnectPrefs.lastSyncMs(ctx) }
    LaunchedEffect(hcAvailability) {
        hcGrantedCount = if (hcAvailability == OhdHealthConnect.Availability.Installed) {
            runCatching { OhdHealthConnect.grantedPermissions(ctx).size }.getOrDefault(0)
        } else {
            0
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        OhdTopBar(title = "Sources", onBack = onBack)

        // ---- This device ----
        OhdSectionHeader("THIS DEVICE")
        OhdListItem(
            primary = "OHD Connect (this phone)",
            secondary = "${Build.MANUFACTURER} ${Build.MODEL} · Android ${Build.VERSION.RELEASE}",
            meta = "$eventCount events",
            onClick = {
                Toast
                    .makeText(ctx, "Device details — coming soon", Toast.LENGTH_SHORT)
                    .show()
            },
        )

        // ---- Connected sources ----
        OhdSectionHeader("CONNECTED SOURCES")
        OhdListItem(
            primary = "Health Connect",
            secondary = healthConnectSecondary(hcAvailability, hcGrantedCount, hcLastSyncMs),
            meta = "›",
            leading = {
                Icon(
                    imageVector = OhdIcons.Activity,
                    contentDescription = null,
                    tint = OhdColors.Red,
                    modifier = Modifier.size(20.dp),
                )
            },
            onClick = onOpenHealthConnect,
        )

        // ---- Action buttons ----
        Spacer(Modifier.height(20.dp))
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(modifier = Modifier.weight(1f)) {
                OhdButton(
                    label = "Add source",
                    variant = OhdButtonVariant.Ghost,
                    onClick = {
                        Toast
                            .makeText(
                                ctx,
                                "Pairing UI coming soon — for now use Settings → Health Connect",
                                Toast.LENGTH_SHORT,
                            )
                            .show()
                    },
                )
            }
            Box(modifier = Modifier.weight(1f)) {
                OhdButton(
                    label = "Import data",
                    variant = OhdButtonVariant.Primary,
                    onClick = onImportData,
                )
            }
        }

        // ---- Coming soon footer ----
        Spacer(Modifier.height(16.dp))
        OhdDivider()
        Spacer(Modifier.height(12.dp))
        Text(
            text = "Paired wearables, web sessions, and clinic-issued tokens will appear here once pairing flows ship. Use Import data to load one-off CSV / JSONL exports.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        )
        Spacer(Modifier.height(24.dp))
    }
}

/**
 * Secondary line for the Health Connect entry. Centralised so the exact
 * labels are easy to grep.
 */
private fun healthConnectSecondary(
    availability: OhdHealthConnect.Availability,
    grantedCount: Int,
    lastSyncMs: Long?,
): String = when (availability) {
    OhdHealthConnect.Availability.Installed -> {
        if (grantedCount > 0) {
            val sync = lastSyncMs?.let { fmtRelative(it) } ?: "never"
            "Connected · $grantedCount permissions · last sync $sync"
        } else {
            "Installed · grant access in settings"
        }
    }
    OhdHealthConnect.Availability.NeedsUpdate -> "Needs update"
    OhdHealthConnect.Availability.NotInstalled -> "Not installed"
}
