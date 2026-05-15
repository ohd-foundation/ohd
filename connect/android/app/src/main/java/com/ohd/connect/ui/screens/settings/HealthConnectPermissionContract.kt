package com.ohd.connect.ui.screens.settings

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.health.connect.client.PermissionController

/**
 * Compose-native helper that wraps Health Connect's
 * `PermissionController.createRequestPermissionResultContract()`.
 *
 * The contract is the canonical way for an app to ask the Health Connect
 * provider for read/write permissions. It accepts a `Set<String>` of
 * `android.permission.health.*` strings and returns the granted subset
 * via [onResult].
 *
 * Usage from the Settings screen:
 *
 *   val launcher = rememberHealthConnectPermissionLauncher { granted ->
 *       // recompute "X of Y" + persist if you care
 *   }
 *   OhdButton(label = "Grant access", onClick = { launcher.launch(OhdHealthConnect.PermissionsRead) })
 *
 * Why a separate file: this is the only place we touch
 * `androidx.activity.compose.rememberLauncherForActivityResult`, which
 * has a small but specific lifecycle dependency on `ComponentActivity`.
 * Pulling it into its own file keeps the screen Composable from having
 * to import any Activity APIs and keeps both files trivially testable.
 */
@Composable
fun rememberHealthConnectPermissionLauncher(
    onResult: (granted: Set<String>) -> Unit,
): ActivityResultLauncher<Set<String>> {
    // `rememberUpdatedState` so a recomposing parent that passes a new
    // `onResult` lambda doesn't desync from the launcher (the
    // `rememberLauncherForActivityResult` callback is captured once and
    // reused across recompositions; without `rememberUpdatedState` the
    // launcher would call the *first* lambda forever).
    val callback = rememberUpdatedState(onResult)
    val contract = remember { PermissionController.createRequestPermissionResultContract() }
    return rememberLauncherForActivityResult(contract) { granted ->
        callback.value(granted)
    }
}
