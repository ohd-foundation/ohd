package com.ohd.emergency

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument

import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.ui.screens.BreakGlassScreen
import com.ohd.emergency.ui.screens.DiscoveryScreen
import com.ohd.emergency.ui.screens.HandoffScreen
import com.ohd.emergency.ui.screens.InterventionScreen
import com.ohd.emergency.ui.screens.LoginScreen
import com.ohd.emergency.ui.screens.PatientScreen
import com.ohd.emergency.ui.screens.TimelineScreen
import com.ohd.emergency.ui.theme.EmergencyTheme

/**
 * OHD Emergency paramedic tablet — entry point.
 *
 * Single-Activity NavHost. Flow:
 *
 *   /login           → /discovery (after operator OIDC sign-in)
 *   /discovery       → /break-glass/{beaconId} (tap a beacon)
 *   /break-glass/X   → /patient/{caseUlid} (on grant) | back to /discovery (on reject)
 *   /patient/{caseUlid} ↔ /intervention/{caseUlid} ↔ /timeline/{caseUlid} ↔ /handoff/{caseUlid}
 *   /handoff/{caseUlid} → /discovery (after handoff confirmed)
 *
 * Panic-logout from any screen returns to /login and clears the [CaseVault].
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        EmergencyRepository.init(applicationContext)
        setContent {
            EmergencyTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    EmergencyApp()
                }
            }
        }
    }
}

@Composable
private fun EmergencyApp() {
    val ctx = LocalContext.current
    val nav = rememberNavController()
    val scope = rememberCoroutineScope()

    // Re-key the entire NavHost on sign-out so the start destination
    // re-evaluates against the updated OperatorSession.
    var sessionVersion by remember { mutableStateOf(0) }

    val startDest = remember(sessionVersion) {
        if (OperatorSession.isSignedIn(ctx)) Routes.DISCOVERY else Routes.LOGIN
    }

    val onPanicLogout: () -> Unit = {
        EmergencyRepository.panicLogout()
        sessionVersion++
        nav.navigate(Routes.LOGIN) {
            popUpTo(0) { inclusive = true }
            launchSingleTop = true
        }
    }

    NavHost(
        navController = nav,
        startDestination = startDest,
    ) {
        composable(Routes.LOGIN) {
            LoginScreen(
                onSignedIn = {
                    sessionVersion++
                    nav.navigate(Routes.DISCOVERY) {
                        popUpTo(Routes.LOGIN) { inclusive = true }
                        launchSingleTop = true
                    }
                },
            )
        }

        composable(Routes.DISCOVERY) {
            DiscoveryScreen(
                onPickBeacon = { beacon ->
                    nav.navigate(Routes.breakGlass(beacon.beaconId))
                },
                onResumeCase = { caseUlid ->
                    nav.navigate(Routes.patient(caseUlid))
                },
                onPanicLogout = onPanicLogout,
            )
        }

        composable(
            route = Routes.breakGlassPattern(),
            arguments = listOf(navArgument(Routes.ARG_BEACON_ID) { type = NavType.StringType }),
        ) { backStack ->
            val raw = backStack.arguments?.getString(Routes.ARG_BEACON_ID) ?: ""
            val beaconId = Routes.decodeBeaconId(raw)
            BreakGlassScreen(
                beaconId = beaconId,
                onApproved = { caseUlid ->
                    nav.navigate(Routes.patient(caseUlid)) {
                        popUpTo(Routes.DISCOVERY) { inclusive = false }
                        launchSingleTop = true
                    }
                },
                onCancelled = {
                    nav.popBackStack(route = Routes.DISCOVERY, inclusive = false)
                },
            )
        }

        composable(
            route = Routes.patientPattern(),
            arguments = listOf(navArgument(Routes.ARG_CASE_ULID) { type = NavType.StringType }),
        ) { backStack ->
            val caseUlid = backStack.arguments?.getString(Routes.ARG_CASE_ULID) ?: ""
            PatientScreen(
                caseUlid = caseUlid,
                onOpenIntervention = {
                    nav.navigate(Routes.intervention(caseUlid)) { launchSingleTop = true }
                },
                onOpenTimeline = {
                    nav.navigate(Routes.timeline(caseUlid)) { launchSingleTop = true }
                },
                onOpenHandoff = {
                    nav.navigate(Routes.handoff(caseUlid)) { launchSingleTop = true }
                },
                onPanicLogout = onPanicLogout,
            )
        }

        composable(
            route = Routes.interventionPattern(),
            arguments = listOf(navArgument(Routes.ARG_CASE_ULID) { type = NavType.StringType }),
        ) { backStack ->
            val caseUlid = backStack.arguments?.getString(Routes.ARG_CASE_ULID) ?: ""
            InterventionScreen(
                caseUlid = caseUlid,
                onOpenPatient = {
                    nav.navigate(Routes.patient(caseUlid)) { launchSingleTop = true }
                },
                onOpenTimeline = {
                    nav.navigate(Routes.timeline(caseUlid)) { launchSingleTop = true }
                },
                onOpenHandoff = {
                    nav.navigate(Routes.handoff(caseUlid)) { launchSingleTop = true }
                },
                onPanicLogout = onPanicLogout,
            )
        }

        composable(
            route = Routes.timelinePattern(),
            arguments = listOf(navArgument(Routes.ARG_CASE_ULID) { type = NavType.StringType }),
        ) { backStack ->
            val caseUlid = backStack.arguments?.getString(Routes.ARG_CASE_ULID) ?: ""
            TimelineScreen(
                caseUlid = caseUlid,
                onOpenPatient = {
                    nav.navigate(Routes.patient(caseUlid)) { launchSingleTop = true }
                },
                onOpenIntervention = {
                    nav.navigate(Routes.intervention(caseUlid)) { launchSingleTop = true }
                },
                onOpenHandoff = {
                    nav.navigate(Routes.handoff(caseUlid)) { launchSingleTop = true }
                },
                onPanicLogout = onPanicLogout,
            )
        }

        composable(
            route = Routes.handoffPattern(),
            arguments = listOf(navArgument(Routes.ARG_CASE_ULID) { type = NavType.StringType }),
        ) { backStack ->
            val caseUlid = backStack.arguments?.getString(Routes.ARG_CASE_ULID) ?: ""
            HandoffScreen(
                caseUlid = caseUlid,
                onComplete = {
                    com.ohd.emergency.data.CaseVault.clear()
                    nav.navigate(Routes.DISCOVERY) {
                        popUpTo(Routes.DISCOVERY) { inclusive = true }
                        launchSingleTop = true
                    }
                },
                onOpenPatient = {
                    nav.navigate(Routes.patient(caseUlid)) { launchSingleTop = true }
                },
                onOpenIntervention = {
                    nav.navigate(Routes.intervention(caseUlid)) { launchSingleTop = true }
                },
                onOpenTimeline = {
                    nav.navigate(Routes.timeline(caseUlid)) { launchSingleTop = true }
                },
                onPanicLogout = onPanicLogout,
            )
        }
    }

    // scope reserved for future foreground-service launches; suppress lint.
    @Suppress("UNUSED_VARIABLE")
    val _scope = scope
}
