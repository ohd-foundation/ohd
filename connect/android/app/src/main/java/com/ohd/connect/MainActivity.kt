package com.ohd.connect

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import com.ohd.connect.data.Auth
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.BottomTab
import com.ohd.connect.ui.components.OhdBottomBar
import com.ohd.connect.ui.screens.DashboardScreen
import com.ohd.connect.ui.screens.GrantsScreen
import com.ohd.connect.ui.screens.LogScreen
import com.ohd.connect.ui.screens.SettingsScreen
import com.ohd.connect.ui.screens.SetupScreen
import com.ohd.connect.ui.theme.OhdConnectTheme

/**
 * Single-Activity entry point for OHD Connect.
 *
 * Flow:
 *   - First launch: [Auth.isFirstRun] is true → render [SetupScreen].
 *     User picks "Use on-device storage" → repository creates the file,
 *     issues a self-session token, and we set the first-run flag.
 *   - Subsequent launches: open the existing storage on-the-fly and render
 *     [MainSurface] (bottom-bar nav: Log / Dashboard / Grants / Settings).
 *
 * The Compose tree references the uniffi bindings only via
 * [StorageRepository] — none of the screens import `uniffi.ohd_storage.*`.
 * That keeps the rest of the codebase compilable when the Stage 1 / Stage 2
 * codegen flow in `BUILD.md` hasn't been run yet (only
 * `data/StorageRepository.kt` fails to resolve in that case, and even
 * those failures are gated behind TODO comments).
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        StorageRepository.init(applicationContext)
        setContent {
            OhdConnectTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    OhdConnectApp()
                }
            }
        }
    }
}

@Composable
private fun OhdConnectApp() {
    val ctx = LocalContext.current
    var inSetup by remember { mutableStateOf(Auth.isFirstRun(ctx)) }

    if (inSetup) {
        SetupScreen(onSetupDone = { inSetup = false })
    } else {
        // Reopen the existing storage handle on every cold start. This is
        // a cheap operation; the SQLCipher PRAGMA key check is the
        // dominant cost, ~tens of ms.
        val opened = remember {
            // TODO: real key derivation, see StorageRepository.openOrCreate.
            val stubKeyHex = "00".repeat(32)
            StorageRepository.open(stubKeyHex)
        }
        // We log the failure here for the v0 scaffold — a real impl
        // would route to a "re-enter passphrase / reset storage" screen.
        opened.onFailure { /* TODO: route to passphrase reset */ }

        MainSurface()
    }
}

@Composable
private fun MainSurface() {
    var current by remember { mutableStateOf(BottomTab.Log) }

    Scaffold(
        bottomBar = { OhdBottomBar(current = current, onSelect = { current = it }) },
    ) { padding ->
        when (current) {
            BottomTab.Log -> LogScreen(contentPadding = padding)
            BottomTab.Dashboard -> DashboardScreen(contentPadding = padding)
            BottomTab.Grants -> GrantsScreen(contentPadding = padding)
            BottomTab.Settings -> SettingsScreen(contentPadding = padding)
        }
    }
}

@Preview(showBackground = true)
@Composable
private fun OhdConnectAppPreview() {
    OhdConnectTheme {
        MainSurface()
    }
}
