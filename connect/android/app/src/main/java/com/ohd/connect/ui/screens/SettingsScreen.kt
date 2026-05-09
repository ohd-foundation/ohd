package com.ohd.connect.ui.screens

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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ohd.connect.BuildConfig
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.MonoStyle

/**
 * Settings tab — navigation root for the secondary surfaces:
 *  - Storage / identity (the in-tab cards on the root)
 *  - Emergency / Break-glass → [EmergencySettingsScreen]
 *  - Cases → [CasesScreen]
 *  - Audit → [AuditScreen]
 *  - Export / portability → [ExportScreen]
 *
 * The bottom-bar still has four primary tabs (Log / Dashboard / Grants /
 * Settings) per `ux-design.md`. Cases and Audit are surfaced under
 * Settings rather than as their own top-level tabs because the canonical
 * spec puts them under Settings (`connect/SPEC.md` "Connect surfaces").
 *
 * Sub-navigation is implemented with a tiny in-screen state machine so we
 * can avoid pulling in `androidx.navigation.compose` for v0 — the four
 * sub-screens have no deep-link requirements yet. When they grow, swap
 * for a real NavHost.
 */
@Composable
fun SettingsScreen(contentPadding: PaddingValues) {
    var route by remember { mutableStateOf(SettingsRoute.Root) }

    when (route) {
        SettingsRoute.Root -> SettingsRoot(
            contentPadding = contentPadding,
            onNavigate = { route = it },
        )
        SettingsRoute.Emergency -> SubScreen(
            title = "Emergency / Break-glass",
            onBack = { route = SettingsRoute.Root },
            contentPadding = contentPadding,
        ) { padding -> EmergencySettingsScreen(contentPadding = padding) }
        SettingsRoute.Cases -> SubScreen(
            title = "Cases",
            onBack = { route = SettingsRoute.Root },
            contentPadding = contentPadding,
        ) { padding -> CasesScreen(contentPadding = padding) }
        SettingsRoute.Audit -> SubScreen(
            title = "Audit",
            onBack = { route = SettingsRoute.Root },
            contentPadding = contentPadding,
        ) { padding -> AuditScreen(contentPadding = padding) }
        SettingsRoute.Export -> SubScreen(
            title = "Export",
            onBack = { route = SettingsRoute.Root },
            contentPadding = contentPadding,
        ) { padding -> ExportScreen(contentPadding = padding) }
    }
}

private enum class SettingsRoute { Root, Emergency, Cases, Audit, Export }

@Composable
private fun SettingsRoot(
    contentPadding: PaddingValues,
    onNavigate: (SettingsRoute) -> Unit,
) {
    val identity = remember { StorageRepository.identity() }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Settings",
                style = MaterialTheme.typography.headlineSmall,
            )

            Spacer(Modifier.height(8.dp))

            Text(
                text = "Storage",
                style = MaterialTheme.typography.titleMedium,
            )
            SettingRow("Storage path", identity.storagePath, mono = true)
            SettingRow("User ULID", identity.userUlid, mono = true)
            SettingRow(
                label = "Self-session token",
                value = identity.tokenTruncated ?: "(none — re-issue from setup)",
                mono = true,
            )
            SettingRow("Format version", identity.formatVersion)
            SettingRow("Protocol version", identity.protocolVersion)
            SettingRow(
                label = "App build",
                value = "${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})",
            )

            Spacer(Modifier.height(8.dp))
            HorizontalDivider()
            Spacer(Modifier.height(8.dp))

            // Sub-navigation tiles
            NavTile(
                title = "Emergency / Break-glass",
                sub = "Configure first-responder access, BLE beacon, history window, trusted authorities.",
                onClick = { onNavigate(SettingsRoute.Emergency) },
            )
            NavTile(
                title = "Cases",
                sub = "Active and recent cases — emergencies, hospital admissions, clinic visits.",
                onClick = { onNavigate(SettingsRoute.Cases) },
            )
            NavTile(
                title = "Audit",
                sub = "Every read and write under your data, filterable by op kind and time.",
                onClick = { onNavigate(SettingsRoute.Audit) },
            )
            NavTile(
                title = "Export / portability",
                sub = "Full lossless export, doctor PDF, migration assistant.",
                onClick = { onNavigate(SettingsRoute.Export) },
            )

            Spacer(Modifier.height(8.dp))
            HorizontalDivider()
            Spacer(Modifier.height(8.dp))

            Text(
                text = "Coming in v0.x",
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                text = "• Notification preferences and quiet hours\n" +
                        "• Identity bindings and sessions list\n" +
                        "• Health Connect bridge service\n" +
                        "• Real OAuth code+PKCE login\n" +
                        "• OHDC uniffi bindings for cases/audit/export RPCs",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun NavTile(title: String, sub: String, onClick: () -> Unit) {
    OutlinedButton(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
    ) {
        Column(modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(
                sub,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun SubScreen(
    title: String,
    onBack: () -> Unit,
    contentPadding: PaddingValues,
    body: @Composable (PaddingValues) -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            IconButton(onClick = onBack) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                )
            }
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
            )
        }
        body(PaddingValues(0.dp))
    }
}

@Composable
private fun SettingRow(label: String, value: String, mono: Boolean = false) {
    Column(modifier = Modifier.padding(vertical = 4.dp)) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (mono) {
            Text(text = value, style = MonoStyle)
        } else {
            Text(text = value, style = MaterialTheme.typography.bodyMedium)
        }
    }
}
