package com.ohd.connect.ui.nav

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.navigation.NavController
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import com.ohd.connect.ui.components.OhdTab
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.screens.AuditScreen
import com.ohd.connect.ui.screens.CasesScreen
import com.ohd.connect.ui.screens.SourcesScreen
import com.ohd.connect.ui.screens.import_.ImportChooserScreen
import com.ohd.connect.ui.screens.import_.ImportCsvScreen
import com.ohd.connect.ui.screens.import_.ImportJsonlScreen
import com.ohd.connect.ui.screens.import_.ImportSamsungEcgScreen
import com.ohd.connect.ui.screens.EditEventScreen
import com.ohd.connect.ui.screens.EmergencySettingsScreen
import com.ohd.connect.ui.screens.findEventByUlid
import com.ohd.connect.ui.screens.ExportScreen
import com.ohd.connect.ui.screens.FoodDetailScreen
import androidx.compose.ui.platform.LocalContext
import com.ohd.connect.data.BarcodeScanner
import com.ohd.connect.ui.screens.FoodScreen
import com.ohd.connect.ui.screens.FoodSearchScreen
import com.ohd.connect.ui.screens.foodByName
import com.ohd.connect.ui.screens.FormBuilderScreen
import com.ohd.connect.ui.screens.GrantsScreen
import com.ohd.connect.ui.screens.HomeScreen
import com.ohd.connect.ui.screens.MeasurementScreen
import com.ohd.connect.ui.screens.MedicationLibraryScreen
import com.ohd.connect.ui.screens.MedicationScreen
import com.ohd.connect.ui.screens.NotificationsScreen
import com.ohd.connect.ui.screens.OnboardingStorageScreen
import com.ohd.connect.ui.screens.PainScoreScreen
import com.ohd.connect.ui.screens.PendingScreen
import com.ohd.connect.ui.screens.RecentEventsScreen
import com.ohd.connect.ui.screens.SymptomLogScreen
import com.ohd.connect.ui.screens.UrineStripScreen
import com.ohd.connect.ui.screens._shared.QuickMeasureKind
import com.ohd.connect.ui.screens.cord.CordChatScreen
import com.ohd.connect.ui.screens.settings.AboutScreen
import com.ohd.connect.ui.screens.settings.AccessSettingsScreen
import com.ohd.connect.ui.screens.settings.ActivitiesSettingsScreen
import com.ohd.connect.ui.screens.settings.CordSettingsScreen
import com.ohd.connect.ui.screens.settings.FoodSettingsScreen
import com.ohd.connect.ui.screens.settings.FormsSettingsScreen
import com.ohd.connect.ui.screens.settings.HealthConnectSettingsScreen
import com.ohd.connect.ui.screens.settings.LicencesScreen
import com.ohd.connect.ui.screens.settings.RemindersSettingsScreen
import com.ohd.connect.ui.screens.settings.SettingsDestination
import com.ohd.connect.ui.screens.settings.SettingsHubScreen
import com.ohd.connect.ui.screens.settings.StorageSettingsScreen
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.launch

/**
 * Routes for the four-tab consumer flow + per-screen children.
 *
 * Each route's [route] string is what `NavController.navigate(...)` consumes.
 * Routes are flat — no parameters — because every per-screen state today
 * lives inside the screen composable (no detail-by-id flows yet). When the
 * Medication / Food / etc. detail screens grow real IDs we'll switch the
 * loggers to parameterised templates (`"log/medication/{id}"`).
 *
 * The four root tabs ([Home], [LogPicker], [History], [SettingsHub]) are
 * the destinations the bottom bar can reach directly. [LogPicker] is a
 * pseudo-route — see [LogPickerSheet] — used as a placeholder for the
 * "LOG" tab. In practice tapping LOG from the bar opens the modal sheet
 * without changing the back-stack route; only after the user picks a
 * logger does navigation occur.
 */
sealed class OhdRoute(val route: String) {
    // Tab roots
    data object Home : OhdRoute("home")
    data object LogPicker : OhdRoute("log_picker")
    data object History : OhdRoute("history")
    data object SettingsHub : OhdRoute("settings")

    // Loggers
    data object LogMedication : OhdRoute("log/medication")
    data object LogFood : OhdRoute("log/food")
    /**
     * Food search. Optional `?prefill=` query arg lets a barcode scan or
     * deep link land directly on the search screen with a pre-populated
     * input. Empty / missing → start with an empty input.
     */
    data object LogFoodSearch : OhdRoute("log/food/search") {
        const val PATTERN = "log/food/search?prefill={prefill}"
        fun withPrefill(value: String): String =
            "log/food/search?prefill=${java.net.URLEncoder.encode(value, "UTF-8")}"
    }

    /**
     * Food detail — opened from the search results list with a single
     * `name` path arg. The arg is URL-encoded to survive characters like
     * spaces and `—` that show up in dictionary names ("Oat porridge —
     * Quaker"). Use [forName] to build the runtime route safely.
     */
    data class FoodDetail(val name: String) : OhdRoute("food/detail/{name}") {
        companion object {
            const val PATTERN = "food/detail/{name}"
            fun forName(name: String): String =
                "food/detail/" + java.net.URLEncoder.encode(name, "UTF-8")
        }
    }
    data object LogMeasurement : OhdRoute("log/measurement")
    data object LogSymptom : OhdRoute("log/symptom")
    data object LogUrineStrip : OhdRoute("log/urine_strip")
    /** Pain score (NRS) — opened from `Measurement → Custom forms → Pain score`. */
    data object LogPainScore : OhdRoute("log/pain_score")
    data object FormBuilder : OhdRoute("forms/builder")
    /** Medication library — opened from the Medications top-bar "Library" action. */
    data object MedicationLibrary : OhdRoute("medication/library")

    // Settings sub
    data object SettingsStorage : OhdRoute("settings/storage")
    data object SettingsAccess : OhdRoute("settings/access")
    data object SettingsForms : OhdRoute("settings/forms")
    data object SettingsFood : OhdRoute("settings/food")
    data object SettingsHealthConnect : OhdRoute("settings/health_connect")
    data object SettingsActivities : OhdRoute("settings/activities")
    data object SettingsReminders : OhdRoute("settings/reminders")
    /**
     * CORD settings — landing page when the user taps the CORD row in the
     * Settings hub. Hosts the BYO-provider API key fields, the preferred-model
     * picker, the stub-responses toggle, and the "Sign up for OHD-managed"
     * affordance. The chat surface stays at [CordChat]; this screen's top bar
     * carries an "Open chat" action that navigates there.
     */
    data object SettingsCord : OhdRoute("settings/cord")
    /**
     * About — app identity card + entry points into the licences list,
     * the repo, and the spec. Reached from the new "About & licences"
     * row at the bottom of the Settings hub.
     */
    data object SettingsAbout : OhdRoute("settings/about")
    /** Open-source licences list. Pushed from [SettingsAbout]. */
    data object SettingsLicences : OhdRoute("settings/about/licences")
    /** Profile sub-routes — recovery code grid, plan, linked OIDC identities. */
    data object SettingsProfileRecovery : OhdRoute("settings/profile/recovery")
    data object SettingsProfilePlan : OhdRoute("settings/profile/plan")
    data object SettingsProfileIdentities : OhdRoute("settings/profile/identities")

    // Operator (reached from Settings → Access)
    data object OperatorGrants : OhdRoute("operator/grants")
    data object OperatorPending : OhdRoute("operator/pending")
    data object OperatorCases : OhdRoute("operator/cases")
    data object OperatorAudit : OhdRoute("operator/audit")
    data object OperatorEmergency : OhdRoute("operator/emergency")
    data object OperatorExport : OhdRoute("operator/export")

    /**
     * Edit one previously-logged event — pencil affordance on
     * [com.ohd.connect.ui.screens.RecentEventsScreen]. The ULID is
     * URL-encoded into the route so unusual characters in the ULID (we
     * already use Crockford base32 which is path-safe, but encoding is
     * cheap insurance) survive the back stack.
     */
    data class EditEvent(val ulid: String) : OhdRoute("event/edit/{ulid}") {
        companion object {
            const val PATTERN = "event/edit/{ulid}"
            fun forUlid(ulid: String): String =
                "event/edit/" + java.net.URLEncoder.encode(ulid, "UTF-8")
        }
    }

    // Other
    data object CordChat : OhdRoute("cord")
    data object OnboardingStorage : OhdRoute("onboarding/storage")
    /**
     * Sources — drill-down from the Home "1 source" stat tile. Lists the
     * phone itself plus connected sources (currently just Health Connect;
     * future entries: paired wearables, browser sessions, clinic tokens).
     * Hosts the "Add source" and "Import data" actions.
     */
    data object Devices : OhdRoute("sources")
    data object ImportChooser : OhdRoute("import")
    data object ImportSamsungEcg : OhdRoute("import/samsung-ecg")
    data object ImportCsv : OhdRoute("import/csv")
    data object ImportJsonl : OhdRoute("import/jsonl")
    /**
     * Notifications inbox — destination of the home-header bell icon. The
     * screen reads `NotificationCenter.all(ctx)` and renders the persisted
     * log; `NotificationCenter.append` (called from `RemindersWorker`,
     * Health Connect sync alerts, etc.) is what populates it.
     */
    data object Notifications : OhdRoute("notifications")
}

/** Maps a [OhdTab] to its root [OhdRoute]. */
fun OhdTab.toRoute(): OhdRoute = when (this) {
    OhdTab.Home -> OhdRoute.Home
    OhdTab.Log -> OhdRoute.LogPicker
    OhdTab.History -> OhdRoute.History
    OhdTab.Settings -> OhdRoute.SettingsHub
}

/**
 * Inverse of [toRoute] — only the four tab-root routes resolve to a tab.
 * Nested screens (loggers, settings sub-screens, operator screens) return
 * `null` so the bottom bar knows to hide / not highlight any tab.
 */
fun OhdRoute.toTab(): OhdTab? = when (this) {
    OhdRoute.Home -> OhdTab.Home
    OhdRoute.LogPicker -> OhdTab.Log
    OhdRoute.History -> OhdTab.History
    OhdRoute.SettingsHub -> OhdTab.Settings
    else -> null
}

/** Same mapping but keyed off the raw route string from the back stack. */
fun routeToTab(route: String?): OhdTab? = when (route) {
    OhdRoute.Home.route -> OhdTab.Home
    OhdRoute.LogPicker.route -> OhdTab.Log
    OhdRoute.History.route -> OhdTab.History
    OhdRoute.SettingsHub.route -> OhdTab.Settings
    else -> null
}

/**
 * Returns true when the given back-stack route is one of the four tab
 * roots. Used by [com.ohd.connect.MainActivity] to gate the bottom-bar
 * visibility — nested loggers / settings sub-screens / operator screens
 * hide the bar.
 */
fun isRootRoute(route: String?): Boolean = routeToTab(route) != null

/**
 * Modal "LOG" picker — bottom sheet with four quick-log icons matching the
 * Pencil home grid (Pill / Utensils / Activity / Thermometer).
 *
 * The picker doesn't change route on its own; it dismisses then routes the
 * caller to the chosen logger via [onPick]. Used by the bottom-tab bar's
 * LOG button.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LogPickerSheet(
    onDismiss: () -> Unit,
    onPick: (OhdRoute) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()

    val dismissThen: (OhdRoute) -> Unit = { route ->
        scope.launch {
            sheetState.hide()
        }.invokeOnCompletion {
            onDismiss()
            onPick(route)
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = OhdColors.Bg,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "QUICK LOG",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 11.sp,
                letterSpacing = 2.sp,
                color = OhdColors.Muted,
                modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
            )
            LogPickerRow(
                icon = OhdIcons.Pill,
                label = "Medication",
                onClick = { dismissThen(OhdRoute.LogMedication) },
            )
            LogPickerRow(
                icon = OhdIcons.Utensils,
                label = "Food",
                onClick = { dismissThen(OhdRoute.LogFood) },
            )
            LogPickerRow(
                icon = OhdIcons.Activity,
                label = "Measurement",
                onClick = { dismissThen(OhdRoute.LogMeasurement) },
            )
            LogPickerRow(
                icon = OhdIcons.Thermometer,
                label = "Symptom",
                onClick = { dismissThen(OhdRoute.LogSymptom) },
            )
            Spacer(Modifier.height(8.dp))
        }
    }
}

@Composable
private fun LogPickerRow(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(56.dp)
            .clickable { onClick() }
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = OhdColors.Red,
            modifier = Modifier.size(22.dp),
        )
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 15.sp,
            color = OhdColors.Ink,
        )
    }
}

/**
 * Wires every [OhdRoute] to its screen composable.
 *
 * The host is intentionally flat — no parent/child graphs, no deep linking.
 * Each route's lambda translates the screen's callbacks into
 * `navController.navigate(...)` / `popBackStack()` calls. Snackbar messages
 * for not-yet-wired actions ("Logged Metformin", "Scanning isn't wired yet")
 * route through the supplied [snackbar] host so the caller can present a
 * single host at the activity level.
 *
 * The "LOG" tab (route `OhdRoute.LogPicker`) renders nothing — the modal
 * picker sheet is hosted in [com.ohd.connect.MainActivity]. Tapping LOG on
 * the bottom bar opens the sheet without committing a navigation; this
 * route exists only so `routeToTab` resolves the tab when the sheet is
 * visible.
 */
@Composable
fun OhdNavHost(
    navController: NavHostController,
    contentPadding: PaddingValues,
    snackbar: SnackbarHostState,
    modifier: Modifier = Modifier,
    startDestination: String = OhdRoute.Home.route,
) {
    val scope = rememberCoroutineScope()
    val toast: (String) -> Unit = { msg ->
        scope.launch { snackbar.showSnackbar(msg) }
    }

    NavHost(
        navController = navController,
        startDestination = startDestination,
        modifier = modifier,
    ) {
        // -------- Tab roots --------
        composable(OhdRoute.Home.route) {
            HomeScreen(
                contentPadding = contentPadding,
                onOpenCord = { navController.navigate(OhdRoute.CordChat.route) },
                onOpenNotifications = { navController.navigate(OhdRoute.Notifications.route) },
                onOpenSettings = { navController.navigate(OhdRoute.SettingsHub.route) },
                onOpenHistory = { navController.navigate(OhdRoute.History.route) },
                onLogMedication = { navController.navigate(OhdRoute.LogMedication.route) },
                onLogFood = { navController.navigate(OhdRoute.LogFood.route) },
                onLogMeasurement = { navController.navigate(OhdRoute.LogMeasurement.route) },
                onLogSymptom = { navController.navigate(OhdRoute.LogSymptom.route) },
                onOpenDevices = { navController.navigate(OhdRoute.Devices.route) },
                onFavouriteClick = { _, kind ->
                    // `kind` is the stable token persisted in the favourites
                    // JSON ("blood_pressure" / "glucose" / "weight" /
                    // "temperature" / "heart_rate" / "spo2" / "custom").
                    // `parsePreselect` returns null for unsupported tokens —
                    // the user lands on the un-preselected list in that case.
                    val target = if (parsePreselect(kind) != null) {
                        OhdRoute.LogMeasurement.route + "?preselect=$kind"
                    } else {
                        OhdRoute.LogMeasurement.route
                    }
                    navController.navigate(target)
                },
            )
        }
        // LogPicker is a route-only placeholder — content is rendered as a
        // ModalBottomSheet by MainActivity. The body here is empty so that
        // even if the user lands on the route directly nothing crashes.
        composable(OhdRoute.LogPicker.route) {
            // Intentionally empty — sheet is rendered above the scaffold.
        }
        composable(OhdRoute.History.route) {
            RecentEventsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onEdit = { ulid ->
                    navController.navigate(OhdRoute.EditEvent.forUlid(ulid))
                },
            )
        }
        composable(
            route = OhdRoute.EditEvent.PATTERN,
            arguments = listOf(
                navArgument("ulid") {
                    type = NavType.StringType
                    nullable = false
                },
            ),
        ) { entry ->
            val raw = entry.arguments?.getString("ulid").orEmpty()
            val decoded = runCatching {
                java.net.URLDecoder.decode(raw, "UTF-8")
            }.getOrDefault(raw)
            // Storage's uniffi `EventFilterDto` doesn't expose `event_ulids_in`
            // (the field is hardcoded to `vec![]` in the binding) so we fetch
            // a small recent window and find the row client-side. Good enough
            // while edit-from-recent is the only entry point — the row was
            // just shown in RecentEventsScreen, so it lives in the same
            // 200-row window we re-query here.
            val original = remember(decoded) { findEventByUlid(decoded) }
            EditEventScreen(
                original = original,
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onSaved = { msg -> toast(msg) },
                onError = { msg -> toast(msg) },
            )
        }
        composable(OhdRoute.SettingsHub.route) {
            SettingsHubScreen(
                contentPadding = contentPadding,
                onNavigate = { dest ->
                    val target = when (dest) {
                        SettingsDestination.Storage -> OhdRoute.SettingsStorage
                        SettingsDestination.Access -> OhdRoute.SettingsAccess
                        SettingsDestination.Forms -> OhdRoute.SettingsForms
                        SettingsDestination.Food -> OhdRoute.SettingsFood
                        SettingsDestination.HealthConnect -> OhdRoute.SettingsHealthConnect
                        SettingsDestination.Activities -> OhdRoute.SettingsActivities
                        SettingsDestination.Reminders -> OhdRoute.SettingsReminders
                        SettingsDestination.Cord -> OhdRoute.SettingsCord
                        SettingsDestination.About -> OhdRoute.SettingsAbout
                    }
                    navController.navigate(target.route)
                },
            )
        }

        // -------- Loggers --------
        composable(OhdRoute.LogMedication.route) {
            MedicationScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onOpenLibrary = { navController.navigate(OhdRoute.MedicationLibrary.route) },
                onLogMedication = { name ->
                    toast("Logged $name")
                },
                onToast = { msg -> toast(msg) },
            )
        }
        composable(OhdRoute.MedicationLibrary.route) {
            MedicationLibraryScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onPickEntry = { entry ->
                    // Real persistence (writing the entry to the user's
                    // medication list) lands when a dedicated repo exists;
                    // for the beta cut we acknowledge with a snackbar and pop.
                    toast("Added ${entry.name} to on-hand")
                    navController.popBackStack()
                },
            )
        }
        composable(OhdRoute.LogFood.route) {
            FoodScreen(
                onBack = { navController.popBackStack() },
                onScannedBarcode = { code ->
                    navController.navigate(OhdRoute.LogFoodSearch.withPrefill(code))
                },
                onOpenSearch = { navController.navigate(OhdRoute.LogFoodSearch.route) },
                onOpenEvent = { ulid -> navController.navigate(OhdRoute.EditEvent.forUlid(ulid)) },
                onToast = { msg -> toast(msg) },
                contentPadding = contentPadding,
            )
        }
        composable(
            route = OhdRoute.LogFoodSearch.PATTERN,
            arguments = listOf(
                navArgument("prefill") {
                    type = NavType.StringType
                    defaultValue = ""
                    nullable = false
                },
            ),
        ) { entry ->
            val rawPrefill = entry.arguments?.getString("prefill").orEmpty()
            val prefill = runCatching {
                java.net.URLDecoder.decode(rawPrefill, "UTF-8")
            }.getOrDefault(rawPrefill)
            val ctx = LocalContext.current
            FoodSearchScreen(
                onBack = { navController.popBackStack() },
                // The 44×44 scan button on the search row now just pops back
                // to FoodScreen where the inline CameraX preview lives.
                // Earlier versions launched the fullscreen Google Code
                // Scanner here, which felt like a regression — user wants
                // the same inline preview they saw when first opening Food.
                onScanReturn = { navController.popBackStack() },
                onPickFood = { item ->
                    navController.navigate(OhdRoute.FoodDetail.forName(item.name))
                },
                initialQuery = prefill,
                contentPadding = contentPadding,
            )
        }
        composable(
            route = OhdRoute.FoodDetail.PATTERN,
            arguments = listOf(
                navArgument("name") {
                    type = NavType.StringType
                    nullable = false
                },
            ),
        ) { entry ->
            val raw = entry.arguments?.getString("name").orEmpty()
            val decoded = runCatching {
                java.net.URLDecoder.decode(raw, "UTF-8")
            }.getOrDefault(raw)
            val item = foodByName(decoded)
            if (item == null) {
                // Defensive — should never trigger in practice because the
                // route is only built from `FoodItem.name`. Pop back so the
                // user isn't stranded on a blank screen.
                LaunchedEffect(decoded) {
                    navController.popBackStack()
                }
            } else {
                FoodDetailScreen(
                    item = item,
                    onBack = { navController.popBackStack() },
                    onLogged = { summary ->
                        toast(summary)
                        // Pop twice: out of the detail screen, then out of
                        // the search screen, landing on FoodScreen which
                        // re-queries `food.eaten` on composition and shows
                        // the new entry in the Recent list.
                        navController.popBackStack()
                        navController.popBackStack()
                    },
                    onError = { msg -> toast(msg) },
                    contentPadding = contentPadding,
                )
            }
        }
        composable(
            route = OhdRoute.LogMeasurement.route + "?preselect={preselect}",
            arguments = listOf(
                navArgument("preselect") {
                    type = NavType.StringType
                    defaultValue = ""
                    nullable = false
                },
            ),
        ) { entry ->
            val preselect = entry.arguments?.getString("preselect").orEmpty()
            MeasurementScreen(
                onBack = { navController.popBackStack() },
                onLog = { toast("Use a quick measure or custom form below") },
                onOpenUrineStrip = { navController.navigate(OhdRoute.LogUrineStrip.route) },
                onOpenPainScore = { navController.navigate(OhdRoute.LogPainScore.route) },
                onToast = { msg -> toast(msg) },
                contentPadding = contentPadding,
                preselectKind = parsePreselect(preselect),
            )
        }
        composable(OhdRoute.LogSymptom.route) {
            SymptomLogScreen(
                onBack = { navController.popBackStack() },
                onLog = { text, severity ->
                    val summary = if (text.isBlank()) "Symptom $severity/5" else text.take(40)
                    toast("Logged: $summary ($severity/5)")
                    navController.popBackStack()
                },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.LogUrineStrip.route) {
            UrineStripScreen(
                onBack = { navController.popBackStack() },
                onLog = { selections ->
                    val n = selections.values.count { it != null }
                    toast("Logged urine strip ($n selections)")
                    navController.popBackStack()
                },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.LogPainScore.route) {
            PainScoreScreen(
                onBack = { navController.popBackStack() },
                onLog = { navController.popBackStack() },
                onToast = { msg -> toast(msg) },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.FormBuilder.route) {
            FormBuilderScreen(
                onBack = { navController.popBackStack() },
                onSaved = { spec ->
                    val title = spec.name.ifBlank { "untitled form" }
                    toast("Saved \"$title\" (${spec.fields.size} fields)")
                    navController.popBackStack()
                },
                contentPadding = contentPadding,
            )
        }

        // -------- Settings sub --------
        composable(OhdRoute.SettingsStorage.route) {
            StorageSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onContinue = { navController.popBackStack() },
                onToast = { msg -> toast(msg) },
            )
        }
        composable(OhdRoute.SettingsAccess.route) {
            AccessSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onOpenGrants = { navController.navigate(OhdRoute.OperatorGrants.route) },
                onOpenPending = { navController.navigate(OhdRoute.OperatorPending.route) },
                onOpenCases = { navController.navigate(OhdRoute.OperatorCases.route) },
                onOpenAudit = { navController.navigate(OhdRoute.OperatorAudit.route) },
                onOpenEmergency = { navController.navigate(OhdRoute.OperatorEmergency.route) },
                onOpenExport = { navController.navigate(OhdRoute.OperatorExport.route) },
                onOpenRecovery = { navController.navigate(OhdRoute.SettingsProfileRecovery.route) },
                onOpenPlan = { navController.navigate(OhdRoute.SettingsProfilePlan.route) },
                onOpenIdentities = { navController.navigate(OhdRoute.SettingsProfileIdentities.route) },
            )
        }
        composable(OhdRoute.SettingsForms.route) {
            FormsSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }
        composable(OhdRoute.SettingsFood.route) {
            FoodSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }
        composable(OhdRoute.SettingsHealthConnect.route) {
            HealthConnectSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }
        composable(OhdRoute.SettingsActivities.route) {
            ActivitiesSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onOpenHealthConnect = { navController.navigate(OhdRoute.SettingsHealthConnect.route) },
            )
        }
        composable(OhdRoute.SettingsReminders.route) {
            RemindersSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }
        composable(OhdRoute.SettingsCord.route) {
            CordSettingsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onOpenChat = { navController.navigate(OhdRoute.CordChat.route) },
            )
        }
        composable(OhdRoute.SettingsAbout.route) {
            AboutScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onOpenLicences = { navController.navigate(OhdRoute.SettingsLicences.route) },
            )
        }
        composable(OhdRoute.SettingsLicences.route) {
            LicencesScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }
        composable(OhdRoute.SettingsProfileRecovery.route) {
            com.ohd.connect.ui.screens.RecoveryCodeScreen(
                contentPadding = contentPadding,
                onAcknowledged = { navController.popBackStack() },
                onBack = { navController.popBackStack() },
                title = "Recovery code",
                primaryLabel = "I saved it",
            )
        }
        composable(OhdRoute.SettingsProfilePlan.route) {
            com.ohd.connect.ui.screens.settings.ProfilePlanScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }
        composable(OhdRoute.SettingsProfileIdentities.route) {
            com.ohd.connect.ui.screens.settings.ProfileIdentitiesScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
            )
        }

        // -------- Operator (wrapped with OhdTopBar so they get a back arrow) --------
        composable(OhdRoute.OperatorGrants.route) {
            OperatorScaffold(
                title = "Grants",
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            ) { inner ->
                GrantsScreen(contentPadding = inner)
            }
        }
        composable(OhdRoute.OperatorPending.route) {
            OperatorScaffold(
                title = "Pending approvals",
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            ) { inner ->
                PendingScreen(contentPadding = inner)
            }
        }
        composable(OhdRoute.OperatorCases.route) {
            OperatorScaffold(
                title = "Cases",
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            ) { inner ->
                CasesScreen(contentPadding = inner)
            }
        }
        composable(OhdRoute.OperatorAudit.route) {
            OperatorScaffold(
                title = "Audit log",
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            ) { inner ->
                AuditScreen(contentPadding = inner)
            }
        }
        composable(OhdRoute.OperatorEmergency.route) {
            OperatorScaffold(
                title = "Emergency",
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            ) { inner ->
                EmergencySettingsScreen(
                    contentPadding = inner,
                    onToast = { msg -> toast(msg) },
                )
            }
        }
        composable(OhdRoute.OperatorExport.route) {
            OperatorScaffold(
                title = "Export",
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            ) { inner ->
                ExportScreen(contentPadding = inner)
            }
        }

        // -------- Other --------
        composable(OhdRoute.CordChat.route) {
            CordChatScreen(
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.OnboardingStorage.route) {
            // Visual variant only — first-run actually flows through
            // SetupScreen (option (a) in the migration brief). Kept as a
            // route so we can preview the design in isolation.
            OnboardingStorageScreen(
                onContinue = { _ -> navController.popBackStack() },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.Devices.route) {
            SourcesScreen(
                onBack = { navController.popBackStack() },
                onOpenHealthConnect = { navController.navigate(OhdRoute.SettingsHealthConnect.route) },
                onImportData = { navController.navigate(OhdRoute.ImportChooser.route) },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.ImportChooser.route) {
            ImportChooserScreen(
                onBack = { navController.popBackStack() },
                onSamsungEcg = { navController.navigate(OhdRoute.ImportSamsungEcg.route) },
                onGenericCsv = { navController.navigate(OhdRoute.ImportCsv.route) },
                onGenericJsonl = { navController.navigate(OhdRoute.ImportJsonl.route) },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.ImportSamsungEcg.route) {
            ImportSamsungEcgScreen(
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.ImportCsv.route) {
            ImportCsvScreen(
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.ImportJsonl.route) {
            ImportJsonlScreen(
                onBack = { navController.popBackStack() },
                contentPadding = contentPadding,
            )
        }
        composable(OhdRoute.Notifications.route) {
            // Bell-icon destination. The screen forwards `actionRoute`
            // strings (e.g. "log/medication", "history") via `onNavigate`;
            // we route them through `navController.navigate(...)` so the
            // back-stack stays consistent with the rest of the graph.
            NotificationsScreen(
                contentPadding = contentPadding,
                onBack = { navController.popBackStack() },
                onNavigate = { route -> navController.navigate(route) },
            )
        }
    }
}

/**
 * Wraps an existing operator screen (Grants / Pending / Cases / Audit /
 * Emergency / Export) with an [OhdTopBar] that supplies a back arrow.
 *
 * The operator screens were authored before the redesign and ship their
 * own internal scaffolding (Material3 `Surface`, headline text, …). Per the
 * migration brief the new shell adds a top bar so the user can pop back to
 * Settings → Access. We pass through `PaddingValues.Zero` to the inner
 * screen because the top bar already eats the activity-level padding;
 * giving the operator screen the same padding would double-up.
 */
@Composable
private fun OperatorScaffold(
    title: String,
    onBack: () -> Unit,
    contentPadding: PaddingValues,
    body: @Composable (PaddingValues) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = title, onBack = onBack)
        Box(modifier = Modifier.fillMaxSize()) {
            body(PaddingValues(0.dp))
        }
    }
}

/**
 * Centralised helper to navigate a [NavController] to one of the four tab
 * roots, popping the back stack to the start so we don't accumulate a deep
 * stack as the user toggles between tabs.
 */
fun NavController.navigateToTab(tab: OhdTab) {
    val route = tab.toRoute().route
    navigate(route) {
        // Pop to the start destination so the tab stack stays at most one
        // entry deep — standard Compose-Nav bottom-bar pattern.
        popUpTo(graph.startDestinationId) {
            saveState = true
        }
        launchSingleTop = true
        restoreState = true
    }
}

// =============================================================================
// Measurement preselect helpers
//
// Home favourites tap → `LogMeasurement?preselect=<token>`. The token is
// a stable kebab-style string so the route is human-readable in logs and
// crash reports.
// =============================================================================

/**
 * Map a Home-screen favourite chip label (e.g. "Glucose", "Blood pressure")
 * to the preselect token consumed by the `LogMeasurement` route.
 *
 * Returns `null` for labels that don't yet have a quick-measure kind —
 * the caller should fall back to opening the measurement logger without
 * a preselect.
 */
internal fun favouriteToPreselect(label: String): String? = when (label.trim().lowercase()) {
    "glucose" -> "glucose"
    "blood pressure" -> "blood_pressure"
    "body weight", "weight" -> "weight"
    "body temperature", "temperature" -> "temperature"
    else -> null
}

/**
 * Inverse of [favouriteToPreselect] — parse the route's `?preselect=`
 * value back into a [QuickMeasureKind]. Empty / unknown strings return
 * `null`, which means "no preselect; show the QUICK MEASURES list".
 */
internal fun parsePreselect(token: String): QuickMeasureKind? = when (token.trim().lowercase()) {
    "glucose" -> QuickMeasureKind.Glucose
    "blood_pressure" -> QuickMeasureKind.BloodPressure
    "weight" -> QuickMeasureKind.BodyWeight
    "temperature" -> QuickMeasureKind.BodyTemperature
    else -> null
}
