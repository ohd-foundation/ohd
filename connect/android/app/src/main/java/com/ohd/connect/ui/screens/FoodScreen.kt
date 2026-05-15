package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.EventVisibility
import com.ohd.connect.data.OhdEvent
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.BarcodePreview
import com.ohd.connect.ui.components.NutriStatus
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdNutriGauge
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

/**
 * Food v3 — Pencil `gZMmO.png`, spec §4.6.
 *
 * Layout (top to bottom):
 *  - [OhdTopBar] "Food" with back arrow, no right action.
 *  - [FoodNutritionPanel] — today's macro gauges, fed by `food.eaten` events.
 *  - 207 dp scan area placeholder (camera not wired in v1).
 *  - Search row that taps into [FoodSearchScreen].
 *  - Recent section — `food.eaten` events from today, most recent first.
 */
@Composable
fun FoodScreen(
    onBack: () -> Unit,
    onScannedBarcode: (String) -> Unit,
    onOpenSearch: () -> Unit,
    onOpenEvent: (String) -> Unit = {},
    onToast: (String) -> Unit = {},
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    // Re-fetch on every composition so popping back from FoodDetailScreen
    // (which appends a new event) shows the freshly-logged row immediately.
    // The query is cheap — the limit of 100 caps the read at a worst-case
    // single-day's logging volume.
    var todaysFoods by remember { mutableStateOf<List<OhdEvent>>(emptyList()) }
    // Hidden `intake.*` child events for today — these carry the macros.
    // Aggregated separately from the parent `food.eaten` rows.
    var todaysIntakeChildren by remember { mutableStateOf<List<OhdEvent>>(emptyList()) }
    // In-progress consumptions (bug #5): started events lacking a matching
    // finished event with the same correlation_id. Pulled today + last day
    // since open-overnight gradual consumption (e.g. a Red Bull at 23:00)
    // should remain finishable in the morning.
    var inProgress by remember { mutableStateOf<List<InProgressFood>>(emptyList()) }
    var refreshTick by remember { mutableStateOf(0) }
    // Bump the tick on every screen-resume so navigating back from
    // FoodDetailScreen (where a new food was logged) re-runs the query and
    // the gauges update. Without this, FoodScreen's LaunchedEffect would
    // fire only once per composition and the totals would stay stale at 0.
    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    LaunchedEffect(lifecycleOwner) {
        lifecycleOwner.lifecycle.addObserver(
            androidx.lifecycle.LifecycleEventObserver { _, event ->
                if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
                    refreshTick++
                }
            },
        )
    }
    LaunchedEffect(refreshTick) {
        val finishedFromMs = startOfTodayMs() - 86_400_000L
        val finishedEvents = StorageRepository
            .queryEvents(
                EventFilter(
                    fromMs = finishedFromMs,
                    eventTypesIn = listOf("food.consumption_finished"),
                    limit = 200,
                ),
            )
            .getOrNull()
            .orEmpty()
        val finishedCorrelationIds = finishedEvents
            .mapNotNull { ev ->
                ev.channels.firstOrNull { it.path == "correlation_id" }
                    ?.let { (it.scalar as? OhdScalar.Text)?.v }
            }
            .toSet()
        val startedEvents = StorageRepository
            .queryEvents(
                EventFilter(
                    fromMs = finishedFromMs,
                    eventTypesIn = listOf("food.consumption_started"),
                    limit = 200,
                ),
            )
            .getOrNull()
            .orEmpty()
        inProgress = startedEvents
            .mapNotNull { ev ->
                val cid = ev.channels.firstOrNull { it.path == "correlation_id" }
                    ?.let { (it.scalar as? OhdScalar.Text)?.v } ?: return@mapNotNull null
                if (cid in finishedCorrelationIds) return@mapNotNull null
                val name = ev.channels.firstOrNull { it.path == CH_NAME }
                    ?.let { (it.scalar as? OhdScalar.Text)?.v } ?: "(unknown food)"
                InProgressFood(
                    correlationId = cid,
                    name = name,
                    startedAtMs = ev.timestampMs,
                    channels = ev.channels.associate { ch ->
                        ch.path to (ch.scalar as? OhdScalar.Real)?.v
                    },
                )
            }
            .sortedByDescending { it.startedAtMs }

        // Today's totals: completed `food.eaten` + finished consumption pairs.
        // We surface the parent rows for the Recent list (one row per meal).
        val eatenToday = StorageRepository
            .queryEvents(
                EventFilter(
                    fromMs = startOfTodayMs(),
                    eventTypesIn = listOf(FOOD_EATEN_EVENT_TYPE),
                    limit = 100,
                    visibility = EventVisibility.TopLevelOnly,
                ),
            )
            .getOrNull()
            ?.sortedByDescending { it.timestampMs }
            ?: emptyList()
        // Today's macros come from the intake.* child events. Each carries
        // `value:real` + `unit:text` and is keyed by event_type. Beta28+:
        // these are hidden children (top_level=false) so we ask for All.
        todaysIntakeChildren = StorageRepository
            .queryEvents(
                EventFilter(
                    fromMs = startOfTodayMs(),
                    eventTypesIn = INTAKE_EVENT_TYPES,
                    limit = 2_000,
                    visibility = EventVisibility.All,
                ),
            )
            .getOrNull()
            .orEmpty()
        // Map finished events back to their started counterparts so the
        // Today totals include gradual-consumption macros (the finished
        // event itself only carries correlation_id, not macros).
        val startedByCid = startedEvents.associateBy { ev ->
            ev.channels.firstOrNull { it.path == "correlation_id" }
                ?.let { (it.scalar as? OhdScalar.Text)?.v }.orEmpty()
        }
        val finishedTodayCorrelated = finishedEvents
            .filter { it.timestampMs >= startOfTodayMs() }
            .mapNotNull { ev ->
                val cid = ev.channels.firstOrNull { it.path == "correlation_id" }
                    ?.let { (it.scalar as? OhdScalar.Text)?.v } ?: return@mapNotNull null
                startedByCid[cid]?.let { startedEv ->
                    // Re-stamp to the finish timestamp so the Recent view
                    // ordering reflects when the user actually completed.
                    startedEv.copy(timestampMs = ev.timestampMs)
                }
            }
        todaysFoods = (eatenToday + finishedTodayCorrelated)
            .sortedByDescending { it.timestampMs }
    }

    val totals = remember(todaysIntakeChildren) { aggregateIntakeChildren(todaysIntakeChildren) }

    // Expansion state for FoodNutritionPanel — hoisted here so we can
    // collapse the camera preview (which takes 210 dp of vertical space)
    // while the user is reading the extended-nutrient breakdown. Per spec:
    // "when expanded, hide the scan area to make room".
    var expandedNutrition by remember { mutableStateOf(false) }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Food", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState()),
        ) {
            FoodNutritionPanel(
                totals = totals,
                expanded = expandedNutrition,
                onToggle = { expandedNutrition = !expandedNutrition },
            )

            // Live camera preview with embedded ML Kit barcode detection.
            // Replaces the static greyscale placeholder. Auto-asks for the
            // CAMERA permission on first composition; on a successful read
            // routes to FoodSearch with the value pre-populated.
            //
            // Height: 210 dp — matches the Pencil spec (207 dp ≈ 210)
            // and gives a comfortable framing area for UPC/EAN barcodes
            // (the dark / light bar contrast needs ~1 cm of vertical
            // pixels on a typical phone to read reliably).
            //
            // Hidden when the nutrition panel is expanded — the user is
            // reading the full breakdown and doesn't need the camera
            // preview competing for vertical space.
            if (!expandedNutrition) {
                BarcodePreview(
                    onScanned = { code -> onScannedBarcode(code) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(210.dp)
                        .clip(RectangleShape)
                        // Lighter neutral gray so the placeholder + the
                        // outside-of-greyscale-frame margins blend with the
                        // white app surface instead of looking like a hard
                        // dark band.
                        .background(Color(0xFFB8B8B8)),
                )
            }

            // Bug #5 — IN PROGRESS section. Shown between the camera and
            // search inputs so an open Red Bull is at the user's thumb.
            if (inProgress.isNotEmpty()) {
                OhdSectionHeader(text = "IN PROGRESS")
                FoodInProgressList(
                    items = inProgress,
                    onFinish = { entry ->
                        val outcome = finishConsumption(entry)
                        when (outcome) {
                            is com.ohd.connect.data.PutEventOutcome.Committed -> {
                                onToast("Finished ${entry.name}.")
                                refreshTick++
                            }
                            is com.ohd.connect.data.PutEventOutcome.Pending -> {
                                onToast("Pending review — finish ${entry.name}")
                                refreshTick++
                            }
                            is com.ohd.connect.data.PutEventOutcome.Error -> {
                                onToast("Couldn't finish: ${outcome.message}")
                            }
                        }
                    },
                )
            }

            // Search row — visual `OhdInput` styling but read-only; the
            // whole row taps into the active search screen.
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                FoodSearchInputStub(
                    placeholder = "Search food or type name…",
                    onClick = onOpenSearch,
                )
            }

            OhdSectionHeader(text = "TODAY")

            FoodTodayList(events = todaysFoods, onOpenEvent = onOpenEvent)
        }
    }
}

/**
 * One row of the IN PROGRESS list (bug #5). Captures the started event's
 * timestamp + macros so the FoodScreen can render "Started HH:mm · NN min ago"
 * and the Finish handler can carry the macros into the finished event if we
 * ever want adjustable %. For v1 the finished event only stores the
 * correlation_id; macros come from the started event at aggregation time.
 */
internal data class InProgressFood(
    val correlationId: String,
    val name: String,
    val startedAtMs: Long,
    /** Real-valued channels from the started event (grams, kcal, …). */
    val channels: Map<String, Double?>,
)

@Composable
private fun FoodInProgressList(
    items: List<InProgressFood>,
    onFinish: (InProgressFood) -> Unit,
) {
    val timeFmt = remember { SimpleDateFormat("HH:mm", Locale.getDefault()) }
    val now = System.currentTimeMillis()
    Column(modifier = Modifier.fillMaxWidth()) {
        items.forEachIndexed { index, entry ->
            val ago = ((now - entry.startedAtMs) / 60_000L).coerceAtLeast(0L)
            val agoText = when {
                ago < 1L -> "just now"
                ago < 60L -> "$ago min ago"
                else -> "${ago / 60L}h ${ago % 60L}m ago"
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = entry.name,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 14.sp,
                        color = OhdColors.Ink,
                    )
                    Text(
                        text = "Started ${timeFmt.format(Date(entry.startedAtMs))} · $agoText",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 12.sp,
                        color = OhdColors.Muted,
                    )
                }
                OhdButton(
                    label = "Finish",
                    onClick = { onFinish(entry) },
                )
            }
            if (index < items.lastIndex) {
                OhdDivider()
            }
        }
    }
}

/**
 * Persist a `food.consumption_finished` event keyed by the started event's
 * `correlation_id`. The actual grams default to whatever the started event
 * recorded — adjustable-grams is left as a TODO for the v1.x polish pass.
 */
private fun finishConsumption(entry: InProgressFood): com.ohd.connect.data.PutEventOutcome {
    val actualGrams = entry.channels["grams"] ?: 0.0
    val input = EventInput(
        timestampMs = System.currentTimeMillis(),
        eventType = "food.consumption_finished",
        channels = listOf(
            EventChannelInput(path = "correlation_id", scalar = OhdScalar.Text(entry.correlationId)),
            EventChannelInput(path = "actual_grams", scalar = OhdScalar.Real(actualGrams)),
        ),
        notes = "Finished ${entry.name}",
    )
    return StorageRepository.putEvent(input).getOrElse { e ->
        com.ohd.connect.data.PutEventOutcome.Error(
            code = "INTERNAL",
            message = e.message ?: e::class.simpleName.orEmpty(),
        )
    }
}

/**
 * Aggregated nutrient totals for today, computed by summing the per-event
 * channels of all `food.eaten` (and finished `food.consumption_started`)
 * events.
 *
 * The first five fields (kcal + four macros) feed the always-visible
 * gauges. The remaining fields are surfaced in the "Show more" panel —
 * older events without those channels simply contribute 0.
 *
 * Macro values stay as `Int` so the gauge row can format them with a `g`
 * suffix directly. Sub-gram micronutrients (mg / mcg) are kept as `Double`
 * to preserve the precision the OFF response carries.
 */
internal data class FoodTotals(
    val kcal: Int,
    val carbsG: Int,
    val proteinG: Int,
    val fatG: Int,
    val sugarG: Int,
    val fiberG: Double = 0.0,
    val saturatedFatG: Double = 0.0,
    val transFatG: Double = 0.0,
    val sodiumMg: Double = 0.0,
    val cholesterolMg: Double = 0.0,
    val potassiumMg: Double = 0.0,
    val calciumMg: Double = 0.0,
    val ironMg: Double = 0.0,
    val vitaminCMg: Double = 0.0,
    val vitaminDMcg: Double = 0.0,
) {
    companion object {
        val Zero = FoodTotals(kcal = 0, carbsG = 0, proteinG = 0, fatG = 0, sugarG = 0)
    }
}

/** Channel name constants — see §4.6 / `EventInput.toDto`. */
internal const val FOOD_EATEN_EVENT_TYPE = "food.eaten"
internal const val CH_NAME = "name"
internal const val CH_GRAMS = "grams"
internal const val CH_KCAL = "kcal"
internal const val CH_CARBS = "carbs_g"
internal const val CH_PROTEIN = "protein_g"
internal const val CH_FAT = "fat_g"
internal const val CH_SUGAR = "sugar_g"

// Extended nutrient channels — opportunistically populated by future
// food.eaten events. Older events lacking these channels contribute 0.
internal const val CH_FIBER = "fiber_g"
internal const val CH_SAT_FAT = "saturated_fat_g"
internal const val CH_TRANS_FAT = "trans_fat_g"
internal const val CH_SODIUM = "sodium_mg"
internal const val CH_CHOLESTEROL = "cholesterol_mg"
internal const val CH_POTASSIUM = "potassium_mg"
internal const val CH_CALCIUM = "calcium_mg"
internal const val CH_IRON = "iron_mg"
internal const val CH_VITAMIN_C = "vitamin_c_mg"
internal const val CH_VITAMIN_D = "vitamin_d_mcg"

/**
 * Sum the per-meal `intake.<nutrient>` child events into a single
 * [FoodTotals] for the day's gauges. Each child carries a single
 * `value:real` channel keyed by its event_type, so the aggregation is a
 * flat dispatch on the type name. Top-level food.eaten parents are
 * ignored — they no longer carry macros after the beta28 split.
 */
internal val INTAKE_EVENT_TYPES = listOf(
    "intake.kcal",
    "intake.carbs_g",
    "intake.protein_g",
    "intake.fat_g",
    "intake.sugar_g",
    "intake.fiber_g",
    "intake.saturated_fat_g",
    "intake.trans_fat_g",
    "intake.sodium_mg",
    "intake.cholesterol_mg",
    "intake.potassium_mg",
    "intake.calcium_mg",
    "intake.iron_mg",
    "intake.vitamin_c_mg",
    "intake.vitamin_d_mcg",
    "intake.caffeine_mg",
)

internal fun aggregateIntakeChildren(events: List<OhdEvent>): FoodTotals {
    val sums = HashMap<String, Double>()
    events.forEach { ev ->
        val v = ev.channels
            .firstOrNull { it.path == "value" }
            ?.let { (it.scalar as? OhdScalar.Real)?.v }
            ?: return@forEach
        sums.merge(ev.eventType, v) { a, b -> a + b }
    }
    return FoodTotals(
        kcal = (sums["intake.kcal"] ?: 0.0).toInt(),
        carbsG = (sums["intake.carbs_g"] ?: 0.0).toInt(),
        proteinG = (sums["intake.protein_g"] ?: 0.0).toInt(),
        fatG = (sums["intake.fat_g"] ?: 0.0).toInt(),
        sugarG = (sums["intake.sugar_g"] ?: 0.0).toInt(),
        fiberG = sums["intake.fiber_g"] ?: 0.0,
        saturatedFatG = sums["intake.saturated_fat_g"] ?: 0.0,
        transFatG = sums["intake.trans_fat_g"] ?: 0.0,
        sodiumMg = sums["intake.sodium_mg"] ?: 0.0,
        cholesterolMg = sums["intake.cholesterol_mg"] ?: 0.0,
        potassiumMg = sums["intake.potassium_mg"] ?: 0.0,
        calciumMg = sums["intake.calcium_mg"] ?: 0.0,
        ironMg = sums["intake.iron_mg"] ?: 0.0,
        vitaminCMg = sums["intake.vitamin_c_mg"] ?: 0.0,
        vitaminDMcg = sums["intake.vitamin_d_mcg"] ?: 0.0,
    )
}

@Suppress("UNUSED")
internal fun aggregateMacros(events: List<OhdEvent>): FoodTotals {
    var kcal = 0.0
    var carbs = 0.0
    var protein = 0.0
    var fat = 0.0
    var sugar = 0.0
    var fiber = 0.0
    var saturatedFat = 0.0
    var transFat = 0.0
    var sodium = 0.0
    var cholesterol = 0.0
    var potassium = 0.0
    var calcium = 0.0
    var iron = 0.0
    var vitaminC = 0.0
    var vitaminD = 0.0
    events.forEach { e ->
        e.channels.forEach { ch ->
            val v = (ch.scalar as? OhdScalar.Real)?.v ?: 0.0
            when (ch.path) {
                CH_KCAL -> kcal += v
                CH_CARBS -> carbs += v
                CH_PROTEIN -> protein += v
                CH_FAT -> fat += v
                CH_SUGAR -> sugar += v
                CH_FIBER -> fiber += v
                CH_SAT_FAT -> saturatedFat += v
                CH_TRANS_FAT -> transFat += v
                CH_SODIUM -> sodium += v
                CH_CHOLESTEROL -> cholesterol += v
                CH_POTASSIUM -> potassium += v
                CH_CALCIUM -> calcium += v
                CH_IRON -> iron += v
                CH_VITAMIN_C -> vitaminC += v
                CH_VITAMIN_D -> vitaminD += v
            }
        }
    }
    return FoodTotals(
        kcal = kcal.toInt(),
        carbsG = carbs.toInt(),
        proteinG = protein.toInt(),
        fatG = fat.toInt(),
        sugarG = sugar.toInt(),
        fiberG = fiber,
        saturatedFatG = saturatedFat,
        transFatG = transFat,
        sodiumMg = sodium,
        cholesterolMg = cholesterol,
        potassiumMg = potassium,
        calciumMg = calcium,
        ironMg = iron,
        vitaminCMg = vitaminC,
        vitaminDMcg = vitaminD,
    )
}

/** Local-timezone start-of-today, in epoch ms. Same convention as HomeScreen. */
internal fun startOfTodayMs(): Long {
    val cal = Calendar.getInstance()
    cal.set(Calendar.HOUR_OF_DAY, 0)
    cal.set(Calendar.MINUTE, 0)
    cal.set(Calendar.SECOND, 0)
    cal.set(Calendar.MILLISECOND, 0)
    return cal.timeInMillis
}

/**
 * Today's food log. Each row is a `food.eaten` event from today; tapping it
 * opens the event detail (where the macros + composition children render and
 * the entry can be corrected). This is the editable day-view the gauges
 * summarise — not a favourites strip.
 */
@Composable
private fun FoodTodayList(
    events: List<OhdEvent>,
    onOpenEvent: (String) -> Unit,
) {
    if (events.isEmpty()) {
        OhdListItem(
            primary = "Nothing logged today",
            secondary = "Scan a barcode or search above to add your first entry.",
        )
        return
    }
    val timeFmt = remember { SimpleDateFormat("HH:mm", Locale.getDefault()) }
    Column(modifier = Modifier.fillMaxWidth()) {
        events.forEachIndexed { index, event ->
            val name = event.channels
                .firstOrNull { it.path == CH_NAME }
                ?.let { (it.scalar as? OhdScalar.Text)?.v }
                ?: "(unknown food)"
            val kcal = event.channels
                .firstOrNull { it.path == CH_KCAL }
                ?.let { (it.scalar as? OhdScalar.Real)?.v?.toInt() }
            val grams = event.channels
                .firstOrNull { it.path == CH_GRAMS }
                ?.let { (it.scalar as? OhdScalar.Real)?.v?.toInt() }
            val time = timeFmt.format(Date(event.timestampMs))
            val detail = buildString {
                append(time)
                if (grams != null) append(" · $grams g")
                if (kcal != null && kcal != 0) append(" · $kcal kcal")
            }
            OhdListItem(
                primary = name,
                secondary = detail,
                meta = "›",
                onClick = { onOpenEvent(event.ulid) },
            )
            if (index < events.lastIndex) {
                OhdDivider()
            }
        }
    }
}

/**
 * Read-only `OhdInput`-styled row that opens the search screen when tapped.
 *
 * Mirrors `OhdInput` visuals (44 dp height, 1.5 dp `ohd-line` border, 8 dp
 * corner radius, padding `[h=12]`, `Inter 14 / muted` placeholder) but is
 * clickable rather than editable.
 */
@Composable
private fun FoodSearchInputStub(
    placeholder: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .height(44.dp)
            .background(
                OhdColors.Bg,
                androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
            )
            .border(
                androidx.compose.foundation.BorderStroke(1.5.dp, OhdColors.Line),
                androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
            )
            .clickable { onClick() }
            .padding(horizontal = 12.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(
            text = placeholder,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 14.sp,
            color = OhdColors.Muted,
        )
    }
}

/**
 * Today's nutrition panel — Pencil `OcxF7` block within `gZMmO`.
 *
 * Used by both [FoodScreen] and [FoodSearchScreen]. Renders a header row
 * (Today + kcal counter), four [OhdNutriGauge]s spread `space_between`,
 * plus an optional expanded body with the extended-nutrient rows
 * (fiber, saturated/trans fat, sodium, cholesterol, potassium, calcium,
 * iron, vitamins C / D).
 *
 * Targets are hardcoded for v1 (1240 → 2000 kcal, 73→110 g carbs, …) but
 * the consumed values come from `StorageRepository.queryEvents`. The
 * default [totals] = [FoodTotals.Zero] keeps the panel renderable from
 * preview / tests where storage isn't open.
 *
 * Expansion state is hoisted by the caller so FoodScreen can collapse its
 * camera area when the user taps "Show more" (see [expanded] / [onToggle]).
 * Callers that don't care about cross-panel coordination can pass `null`
 * for [onToggle]; the panel will manage its own internal expansion state.
 */
@Composable
internal fun FoodNutritionPanel(
    modifier: Modifier = Modifier,
    totals: FoodTotals = FoodTotals.Zero,
    expanded: Boolean? = null,
    onToggle: (() -> Unit)? = null,
) {
    val kcalTarget = 2000
    val carbsTarget = 110
    val proteinTarget = 80
    val fatTarget = 70
    val sugarTarget = 20

    // Either use the hoisted state, or fall back to a local state. We
    // intentionally `remember` regardless so the composable signature
    // stays stable across recompositions.
    var localExpanded by remember { mutableStateOf(false) }
    val isExpanded = expanded ?: localExpanded
    val toggle: () -> Unit = onToggle ?: { localExpanded = !localExpanded }

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Today",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 13.sp,
                color = OhdColors.Ink,
            )
            Text(
                text = "${formatNumber(totals.kcal)} / ${formatNumber(kcalTarget)} kcal",
                fontFamily = OhdMono,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Muted,
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OhdNutriGauge(
                label = "Carbs",
                value = "${totals.carbsG}g",
                target = "${carbsTarget}g",
                percent = pct(totals.carbsG, carbsTarget),
                status = statusFor(totals.carbsG, carbsTarget),
            )
            OhdNutriGauge(
                label = "Protein",
                value = "${totals.proteinG}g",
                target = "${proteinTarget}g",
                percent = pct(totals.proteinG, proteinTarget),
                status = statusFor(totals.proteinG, proteinTarget),
            )
            OhdNutriGauge(
                label = "Fat",
                value = "${totals.fatG}g",
                target = "${fatTarget}g",
                percent = pct(totals.fatG, fatTarget),
                status = statusFor(totals.fatG, fatTarget),
            )
            OhdNutriGauge(
                label = "Sugar",
                value = "${totals.sugarG}g",
                target = "${sugarTarget}g",
                percent = pct(totals.sugarG, sugarTarget),
                status = statusFor(totals.sugarG, sugarTarget),
            )
        }

        if (isExpanded) {
            ExtendedNutrientRows(totals)
        }

        // Centred Show more / Show less toggle.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable { toggle() }
                .padding(vertical = 6.dp),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = if (isExpanded) "Show less ▴" else "Show more ▾",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }

        // Bottom 1 dp ohd-line separator per spec.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
    }
}

/**
 * One row of the extended-nutrient breakdown — label on the left
 * (`Inter 13 / muted`), value+unit on the right (`JetBrains Mono 13 / ink`).
 *
 * Visible at the panel level rather than nested so the same row composable
 * can be reused from the per-100 g detail breakdown.
 */
@Composable
private fun NutrientRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
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

/**
 * Render the grouped extended-nutrient breakdown for [totals]. Groups
 * without any non-zero values are dropped wholesale rather than showing a
 * trail of "0 mg" rows.
 */
@Composable
private fun ExtendedNutrientRows(totals: FoodTotals) {
    val carbGroup = listOfNotNull(
        ("Fiber" to formatGramsRow(totals.fiberG)).takeIf { totals.fiberG > 0.0 },
    )
    val fatGroup = listOfNotNull(
        ("Saturated fat" to formatGramsRow(totals.saturatedFatG))
            .takeIf { totals.saturatedFatG > 0.0 },
        ("Trans fat" to formatGramsRow(totals.transFatG)).takeIf { totals.transFatG > 0.0 },
    )
    val saltGroup = listOfNotNull(
        ("Sodium" to formatMgRow(totals.sodiumMg)).takeIf { totals.sodiumMg > 0.0 },
    )
    val otherGroup = listOfNotNull(
        ("Cholesterol" to formatMgRow(totals.cholesterolMg))
            .takeIf { totals.cholesterolMg > 0.0 },
        ("Potassium" to formatMgRow(totals.potassiumMg)).takeIf { totals.potassiumMg > 0.0 },
        ("Calcium" to formatMgRow(totals.calciumMg)).takeIf { totals.calciumMg > 0.0 },
        ("Iron" to formatMgRow(totals.ironMg)).takeIf { totals.ironMg > 0.0 },
        ("Vitamin C" to formatMgRow(totals.vitaminCMg)).takeIf { totals.vitaminCMg > 0.0 },
        ("Vitamin D" to formatMcgRow(totals.vitaminDMcg)).takeIf { totals.vitaminDMcg > 0.0 },
    )

    val groups = listOf(
        "Carbohydrates" to carbGroup,
        "Fats" to fatGroup,
        "Salts" to saltGroup,
        "Other" to otherGroup,
    ).filter { (_, rows) -> rows.isNotEmpty() }

    if (groups.isEmpty()) {
        Text(
            text = "No additional nutrients recorded yet.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 4.dp),
        )
        return
    }

    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        groups.forEach { (groupTitle, rows) ->
            OhdSectionHeader(text = groupTitle.uppercase())
            rows.forEach { (label, value) -> NutrientRow(label, value) }
        }
    }
}

/** "12.3 g" — drops trailing zero for whole-number totals. */
internal fun formatGramsRow(v: Double): String =
    if (v % 1.0 == 0.0) "${v.toInt()} g" else "%.1f g".format(v)

/** "230 mg" — milligram totals round to integer for display. */
internal fun formatMgRow(v: Double): String =
    if (v >= 10.0) "${v.toInt()} mg" else "%.1f mg".format(v)

/** "2.5 mcg" — vitamin D / similar trace-amount nutrients. */
internal fun formatMcgRow(v: Double): String =
    if (v >= 10.0) "${v.toInt()} mcg" else "%.1f mcg".format(v)

private fun pct(value: Int, target: Int): Int =
    if (target <= 0) 0 else ((value.toLong() * 100L) / target).toInt()

/**
 * Map a value/target pair to the gauge sweep colour.
 *
 * Mirrors the spec §4.6 examples:
 *  - <50 % → Light (de-emphasised)
 *  - 50–100 % → Ok (default muted ring)
 *  - >100 % → Exceeded (red)
 */
private fun statusFor(value: Int, target: Int): NutriStatus {
    val p = pct(value, target)
    return when {
        p > 100 -> NutriStatus.Exceeded
        p < 50 -> NutriStatus.Light
        else -> NutriStatus.Ok
    }
}

private fun formatNumber(n: Int): String =
    if (n < 1000) n.toString() else "%,d".format(n)
