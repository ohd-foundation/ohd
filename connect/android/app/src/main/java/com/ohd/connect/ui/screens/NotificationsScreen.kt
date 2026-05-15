package com.ohd.connect.ui.screens

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import com.ohd.connect.data.NotificationCenter
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Notifications inbox — destination of the home-header bell icon.
 *
 * Three states:
 *
 *  - **Empty** — muted "No notifications yet" + a Ghost "Test notification"
 *    button so the user can sanity-check that POST_NOTIFICATIONS is
 *    granted and the [NotificationCenter] channel is alive.
 *  - **Permission banner** — on API 33+, if `POST_NOTIFICATIONS` is not
 *    granted we render an "Allow notifications" banner above the list
 *    with a button that fires the system permission dialog. The banner
 *    disappears once granted.
 *  - **Populated** — newest-first list of [NotificationCenter.NotificationEntry]
 *    rendered through [OhdListItem]. Tapping a row navigates to the
 *    optional `actionRoute`; rows with no route just no-op.
 *
 * The "Clear" top-bar action drops the entire persisted log. System
 * notifications already delivered to the status bar are not affected —
 * the user dismisses them through the usual Android UI.
 *
 * @param onNavigate Forward an in-app route ("log/medication", "history",
 *   etc.) to the host nav controller. The screen never builds `OhdRoute`
 *   objects directly so it can live in `ui/screens/` without depending on
 *   `ui/nav/`.
 */
@Composable
fun NotificationsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onNavigate: (String) -> Unit,
) {
    val ctx = LocalContext.current
    var entries by remember { mutableStateOf<List<NotificationCenter.NotificationEntry>>(emptyList()) }
    var permissionGranted by remember { mutableStateOf(hasPostNotifications(ctx)) }

    LaunchedEffect(Unit) {
        entries = NotificationCenter.all(ctx)
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) { granted ->
        // The dialog may take a couple of frames to settle; the state
        // flip is what hides the banner on next recomposition.
        permissionGranted = granted
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = "Notifications",
            onBack = onBack,
            action = if (entries.isNotEmpty()) TopBarAction(
                label = "Clear",
                onClick = {
                    NotificationCenter.clear(ctx)
                    entries = emptyList()
                },
            ) else null,
        )

        // Permission banner — only on API 33+ when not granted. Older
        // platforms auto-grant POST_NOTIFICATIONS at install time.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && !permissionGranted) {
            PermissionBanner(
                onAllow = { permissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS) },
            )
        }

        if (entries.isEmpty()) {
            EmptyState(
                onTestNotification = {
                    NotificationCenter.append(
                        ctx = ctx,
                        entry = NotificationCenter.NotificationEntry(
                            id = "test_${System.currentTimeMillis()}",
                            timestampMs = System.currentTimeMillis(),
                            title = "Test notification",
                            body = "If you can see this, OHD Connect can reach the status bar.",
                            kind = NotificationCenter.Kind.TEST,
                        ),
                    )
                    entries = NotificationCenter.all(ctx)
                },
            )
        } else {
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                itemsIndexed(entries) { idx, entry ->
                    OhdListItem(
                        primary = entry.title,
                        secondary = entry.body,
                        meta = fmtRecentTimestamp(entry.timestampMs),
                        onClick = {
                            val route = entry.actionRoute
                            if (route != null) onNavigate(route)
                        },
                    )
                    if (idx < entries.lastIndex) OhdDivider()
                }
            }
        }
    }
}

/**
 * Empty-state body — muted "No notifications yet" line + Ghost "Test
 * notification" button. Centered vertically inside the remaining space
 * below the top bar.
 */
@Composable
private fun EmptyState(
    onTestNotification: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(horizontal = 24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "No notifications yet",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 14.sp,
            color = OhdColors.Muted,
        )
        Spacer(modifier = Modifier.height(16.dp))
        OhdButton(
            label = "Test notification",
            onClick = onTestNotification,
            variant = OhdButtonVariant.Ghost,
        )
    }
}

/**
 * Allow-notifications banner shown above the list on API 33+ when the
 * runtime POST_NOTIFICATIONS permission is missing. The "Allow" button
 * launches the system permission dialog.
 *
 * Styled to match the rest of the chrome: 16 dp horizontal padding, 12 dp
 * vertical padding, thin bottom border so it visually merges with the
 * top-bar hairline.
 */
@Composable
private fun PermissionBanner(
    onAllow: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth().background(OhdColors.Bg)) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Allow notifications",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = "Required so OHD can deliver medication reminders to your status bar.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
            Spacer(modifier = Modifier.width(12.dp))
            OhdButton(
                label = "Allow",
                onClick = onAllow,
                variant = OhdButtonVariant.Ghost,
            )
        }
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
    }
}

private fun hasPostNotifications(ctx: android.content.Context): Boolean {
    // Below API 33 the permission is implicitly granted at install time.
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return true
    return ContextCompat.checkSelfPermission(
        ctx,
        Manifest.permission.POST_NOTIFICATIONS,
    ) == PackageManager.PERMISSION_GRANTED
}
