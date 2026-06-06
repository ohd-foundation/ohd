package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.PutEventOutcome
import com.ohd.connect.data.StorageRepository
import org.json.JSONArray
import org.json.JSONObject
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/**
 * Food detail — opened from [FoodSearchScreen] when the user taps a result.
 *
 * Shows the OpenFoodFacts-flavored description, a chip row to pick amount
 * (package / portion / custom grams), a live macros breakdown (rule of three
 * from `per100g`), and a primary "Log {grams} g" button at the bottom.
 *
 * On successful log:
 *  - calls [onLogged] with a one-line summary so the navigation graph can
 *    surface a snackbar at the activity host;
 *  - the graph then pops twice — out of detail, out of search — landing
 *    the user back on [FoodScreen] with refreshed totals.
 */
@Composable
fun FoodDetailScreen(
    item: FoodItem,
    onBack: () -> Unit,
    onLogged: (summary: String) -> Unit,
    onError: (message: String) -> Unit = {},
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    // Default amount: first declared serving if any, otherwise Custom 100 g.
    // The state covers both the chip selection and the custom grams input —
    // they share a single source of truth so the macros panel can compute
    // against `currentGrams()` regardless of which chip is on.
    val initial: AmountSelection =
        if (item.servings.isNotEmpty()) AmountSelection.Preset(0)
        else AmountSelection.Custom("100")
    var selection by remember { mutableStateOf(initial) }

    val grams = currentGrams(item, selection)
    val macros = remember(grams) { computeMacros(item, grams) }

    // Logging writes one parent event plus several child events. Against
    // remote storage each is a blocking network RPC, so the whole batch must
    // run off the main thread — otherwise the UI freezes (ANR) for the
    // duration. `submitting` guards against a double-tap re-firing the batch.
    val scope = rememberCoroutineScope()
    var submitting by remember { mutableStateOf(false) }

    val onSubmit: () -> Unit = submit@{
        val g = grams ?: return@submit
        if (submitting) return@submit
        submitting = true
        scope.launch(Dispatchers.IO) {
            val outcome = logFood(item, g, macros)
            withContext(Dispatchers.Main) {
                submitting = false
                when (outcome) {
                    is PutEventOutcome.Committed -> {
                        onLogged("Logged ${formatGrams(g)} g ${item.name} — ${macros.kcal} kcal")
                    }
                    is PutEventOutcome.Pending -> {
                        onLogged("Pending review — ${item.name}")
                    }
                    is PutEventOutcome.Error -> {
                        onError("Couldn't log: ${outcome.message}")
                    }
                }
            }
        }
    }

    // Bug #5 — "Start now" gradual consumption flow.
    //
    // Persists a `food.consumption_started` event with the same macros as
    // `food.eaten` plus a fresh `correlation_id` (ULID-shaped string) so the
    // matching `food.consumption_finished` can later close the pair.
    val onStart: () -> Unit = start@{
        val g = grams ?: return@start
        if (submitting) return@start
        submitting = true
        val correlationId = newCorrelationId()
        scope.launch(Dispatchers.IO) {
            val outcome = logFoodStarted(item, g, macros, correlationId)
            withContext(Dispatchers.Main) {
                submitting = false
                when (outcome) {
                    is PutEventOutcome.Committed -> {
                        onLogged("Started ${item.name}. Tap 'Finish' on Food when done.")
                    }
                    is PutEventOutcome.Pending -> {
                        onLogged("Pending review — start ${item.name}")
                    }
                    is PutEventOutcome.Error -> {
                        onError("Couldn't start: ${outcome.message}")
                    }
                }
            }
        }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = item.name,
            onBack = onBack,
            action = TopBarAction(
                label = "Add",
                onClick = onSubmit,
                enabled = grams != null && grams > 0.0,
            ),
        )

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 20.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            // 1. Brand + source line.
            BrandSourceLine(item)

            // 2. Description paragraph.
            Text(
                text = item.description,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 14.sp,
                lineHeight = 21.sp,
                color = OhdColors.Muted,
            )

            // 3. Amount selector — chips + optional custom grams input.
            // SectionHeader carries its own [v=8, h=16] padding; we
            // wrap it with negative horizontal padding to cancel the
            // outer column inset (16 dp on the column + 16 dp on the
            // header = 32 dp). Wrapping in a Box that fills full width
            // and removing the column padding for this slot would be
            // cleaner; for now we let the header double-up the inset.
            OhdSectionHeader(text = "AMOUNT")
            AmountChips(
                item = item,
                selection = selection,
                onSelect = { selection = it },
            )
            if (selection is AmountSelection.Custom) {
                val custom = selection as AmountSelection.Custom
                OhdField(
                    label = "Grams",
                    value = custom.text,
                    onValueChange = { selection = AmountSelection.Custom(it) },
                    placeholder = "100",
                    keyboardType = KeyboardType.Number,
                )
            }

            // 4. Macros breakdown.
            OhdSectionHeader(text = "FOR ${formatGrams(grams ?: 0.0)} G")
            MacrosPanel(macros)

            // 4b. Composition — only shown when OFF gave us anything to
            // surface (additives / allergens / NOVA / Nutri-Score etc).
            if (item.hasCompositionData()) {
                OhdSectionHeader(text = "COMPOSITION")
                CompositionPanel(item)
            }

            // 5. Bottom-anchored CTAs.
            //   Row of two: "Log {g} g" (primary, fully consumed at once)
            //   and "Start now" (ghost, marks gradual consumption with a
            //   matching correlation ID — bug #5).
            Box(modifier = Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OhdButton(
                    label = when {
                        submitting -> "Logging…"
                        grams == null || grams <= 0.0 -> "Enter an amount"
                        else -> "Log ${formatGrams(grams)} g"
                    },
                    onClick = onSubmit,
                    enabled = grams != null && grams > 0.0 && !submitting,
                    modifier = Modifier.weight(1f),
                )
                OhdButton(
                    label = "Start now",
                    onClick = onStart,
                    enabled = grams != null && grams > 0.0 && !submitting,
                    variant = OhdButtonVariant.Ghost,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

/**
 * One of two mutually-exclusive amount sources the user can select. The
 * `Preset` variant indexes into [FoodItem.servings]; the `Custom` variant
 * carries its inline text so a re-selection of a preset chip can fall
 * back to the last-typed value if the user toggles back to custom.
 */
private sealed interface AmountSelection {
    data class Preset(val index: Int) : AmountSelection
    data class Custom(val text: String) : AmountSelection
}

/** Resolve the current grams value for the active selection. */
private fun currentGrams(item: FoodItem, sel: AmountSelection): Double? = when (sel) {
    is AmountSelection.Preset -> item.servings.getOrNull(sel.index)?.grams
    is AmountSelection.Custom -> sel.text.trim().toDoubleOrNull()
}

@Composable
private fun BrandSourceLine(item: FoodItem) {
    val parts = buildList {
        if (!item.brand.isNullOrBlank()) add(item.brand)
        add(item.source)
    }
    Text(
        text = parts.joinToString(" · "),
        fontFamily = OhdBody,
        fontWeight = FontWeight.W400,
        fontSize = 12.sp,
        color = OhdColors.Muted,
    )
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AmountChips(
    item: FoodItem,
    selection: AmountSelection,
    onSelect: (AmountSelection) -> Unit,
) {
    // One chip per declared serving, then an always-on Custom (g) chip.
    // Once you stack more than ~3, equal-weight chips squeeze the labels
    // below readability; instead we let them flow with their natural width
    // using FlowRow so the long names like "Big bottle (2000 g)" don't get
    // truncated and the row wraps cleanly to a second line when needed.
    FlowRow(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item.servings.forEachIndexed { idx, serving ->
            AmountChip(
                label = "${serving.name} (${formatGrams(serving.grams)} g)",
                selected = selection is AmountSelection.Preset && selection.index == idx,
                onClick = { onSelect(AmountSelection.Preset(idx)) },
            )
        }
        AmountChip(
            label = "Custom (g)",
            selected = selection is AmountSelection.Custom,
            onClick = {
                // Carry the existing typed value if we're already on Custom;
                // otherwise seed with "100".
                val seed = (selection as? AmountSelection.Custom)?.text ?: "100"
                onSelect(AmountSelection.Custom(seed))
            },
        )
    }
}

@Composable
private fun AmountChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(8.dp)
    val fillModifier = if (selected) {
        Modifier.background(OhdColors.Ink, shape)
    } else {
        Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
    }
    Box(
        modifier = modifier
            .height(40.dp)
            .then(fillModifier)
            .clickable { onClick() }
            .padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = if (selected) FontWeight.W500 else FontWeight.W400,
            fontSize = 12.sp,
            color = if (selected) OhdColors.Bg else OhdColors.Ink,
            maxLines = 2,
        )
    }
}

/** Macros computed for the chosen amount, rounded for display. */
private data class ResolvedMacros(
    val kcal: Int,
    val carbsG: Double,
    val proteinG: Double,
    val fatG: Double,
    val sugarG: Double,
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
    val caffeineMg: Double = 0.0,
)

private fun computeMacros(item: FoodItem, grams: Double?): ResolvedMacros {
    val g = grams ?: 0.0
    val ratio = g / 100.0
    return ResolvedMacros(
        kcal = (item.per100g.kcal * ratio).toInt(),
        carbsG = roundOne(item.per100g.carbsG * ratio),
        proteinG = roundOne(item.per100g.proteinG * ratio),
        fatG = roundOne(item.per100g.fatG * ratio),
        sugarG = roundOne(item.per100g.sugarG * ratio),
        fiberG = roundOne(item.per100g.fiberG * ratio),
        saturatedFatG = roundOne(item.per100g.saturatedFatG * ratio),
        transFatG = roundOne(item.per100g.transFatG * ratio),
        sodiumMg = roundOne(item.per100g.sodiumMg * ratio),
        cholesterolMg = roundOne(item.per100g.cholesterolMg * ratio),
        potassiumMg = roundOne(item.per100g.potassiumMg * ratio),
        calciumMg = roundOne(item.per100g.calciumMg * ratio),
        ironMg = roundOne(item.per100g.ironMg * ratio),
        vitaminCMg = roundOne(item.per100g.vitaminCMg * ratio),
        vitaminDMcg = roundOne(item.per100g.vitaminDMcg * ratio),
        caffeineMg = roundOne(item.per100g.caffeineMg * ratio),
    )
}

private fun roundOne(v: Double): Double = (v * 10.0).toInt() / 10.0

/** Format grams as integer when whole, one decimal otherwise. */
internal fun formatGrams(g: Double): String =
    if (g % 1.0 == 0.0) g.toInt().toString() else "%.1f".format(g)

@Composable
private fun MacrosPanel(macros: ResolvedMacros) {
    // Locally-managed expansion — the detail screen is short enough that
    // there's nothing else to collapse, so we keep it self-contained.
    var expanded by remember { mutableStateOf(false) }

    // Group the extended-nutrient rows the same way FoodNutritionPanel
    // does, dropping any row whose scaled value is 0. We compute this up
    // front so we only render the "Show more" affordance when there's
    // actually extra data to surface.
    val carbGroup = listOfNotNull(
        ("Fiber" to "${formatGrams(macros.fiberG)} g").takeIf { macros.fiberG > 0.0 },
    )
    val fatGroup = listOfNotNull(
        ("Saturated fat" to "${formatGrams(macros.saturatedFatG)} g")
            .takeIf { macros.saturatedFatG > 0.0 },
        ("Trans fat" to "${formatGrams(macros.transFatG)} g")
            .takeIf { macros.transFatG > 0.0 },
    )
    val saltGroup = listOfNotNull(
        ("Sodium" to formatMgForDetail(macros.sodiumMg)).takeIf { macros.sodiumMg > 0.0 },
    )
    val otherGroup = listOfNotNull(
        ("Cholesterol" to formatMgForDetail(macros.cholesterolMg))
            .takeIf { macros.cholesterolMg > 0.0 },
        ("Potassium" to formatMgForDetail(macros.potassiumMg))
            .takeIf { macros.potassiumMg > 0.0 },
        ("Calcium" to formatMgForDetail(macros.calciumMg)).takeIf { macros.calciumMg > 0.0 },
        ("Iron" to formatMgForDetail(macros.ironMg)).takeIf { macros.ironMg > 0.0 },
        ("Vitamin C" to formatMgForDetail(macros.vitaminCMg))
            .takeIf { macros.vitaminCMg > 0.0 },
        ("Vitamin D" to formatMcgForDetail(macros.vitaminDMcg))
            .takeIf { macros.vitaminDMcg > 0.0 },
        ("Caffeine" to formatMgForDetail(macros.caffeineMg))
            .takeIf { macros.caffeineMg > 0.0 },
    )
    val groups = listOf(
        "Carbohydrates" to carbGroup,
        "Fats" to fatGroup,
        "Salts" to saltGroup,
        "Other" to otherGroup,
    ).filter { (_, rows) -> rows.isNotEmpty() }
    val hasExtended = groups.isNotEmpty()

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, RoundedCornerShape(8.dp))
            .border(BorderStroke(1.dp, OhdColors.Line), RoundedCornerShape(8.dp))
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        MacroRow("Calories", "${macros.kcal} kcal")
        MacroRow("Carbs", "${formatGrams(macros.carbsG)} g")
        MacroRow("Protein", "${formatGrams(macros.proteinG)} g")
        MacroRow("Fat", "${formatGrams(macros.fatG)} g")
        MacroRow("Sugar", "${formatGrams(macros.sugarG)} g")

        if (hasExtended && expanded) {
            groups.forEach { (title, rows) ->
                Text(
                    text = title.uppercase(),
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 11.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier.padding(top = 4.dp),
                )
                rows.forEach { (label, value) -> MacroRow(label, value) }
            }
        }

        if (hasExtended) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { expanded = !expanded }
                    .padding(top = 4.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = if (expanded) "Show less ▴" else "Show more ▾",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }
    }
}

private fun FoodItem.hasCompositionData(): Boolean =
    additives.isNotEmpty() || allergens.isNotEmpty() || traces.isNotEmpty() ||
        ingredientsAnalysis.isNotEmpty() || labels.isNotEmpty() ||
        novaGroup != null || nutriScore != null || ecoScore != null ||
        ingredients.isNotEmpty()

@Composable
private fun CompositionPanel(item: FoodItem) {
    var expanded by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, RoundedCornerShape(8.dp))
            .border(BorderStroke(1.dp, OhdColors.Line), RoundedCornerShape(8.dp))
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        // Score row — always visible when any score is present.
        val scoreLine = buildString {
            item.nutriScore?.let { append("Nutri-Score ").append(it.uppercase()) }
            item.novaGroup?.let {
                if (isNotEmpty()) append("  ·  ")
                append("NOVA ").append(it).append(" (")
                append(novaLabel(it)).append(")")
            }
            item.ecoScore?.let {
                if (isNotEmpty()) append("  ·  ")
                append("Eco-Score ").append(it.uppercase())
            }
        }
        if (scoreLine.isNotEmpty()) {
            Text(
                text = scoreLine,
                fontFamily = OhdMono,
                fontWeight = FontWeight.W500,
                fontSize = 12.sp,
                color = OhdColors.Ink,
            )
        }

        if (item.additives.isNotEmpty()) {
            CompositionRow("Additives (${item.additives.size})", formatAdditives(item.additives))
        }
        if (item.allergens.isNotEmpty()) {
            CompositionRow("Allergens", item.allergens.joinToString(", "))
        }
        if (item.traces.isNotEmpty()) {
            CompositionRow("May contain", item.traces.joinToString(", "))
        }
        if (item.ingredientsAnalysis.isNotEmpty()) {
            CompositionRow("Analysis", item.ingredientsAnalysis.joinToString(", "))
        }

        if (expanded) {
            if (item.labels.isNotEmpty()) {
                CompositionRow("Labels", item.labels.joinToString(", "))
            }
            if (item.ingredients.isNotEmpty()) {
                CompositionRow(
                    "Ingredients (${item.ingredients.size})",
                    item.ingredients.joinToString(", "),
                )
            }
        }

        val hasMore = item.labels.isNotEmpty() || item.ingredients.isNotEmpty()
        if (hasMore) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { expanded = !expanded }
                    .padding(top = 4.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = if (expanded) "Show less ▴" else "Show ingredients ▾",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }
    }
}

@Composable
private fun CompositionRow(label: String, value: String) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            text = label.uppercase(),
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 10.sp,
            color = OhdColors.Muted,
        )
        Text(
            text = value,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
            lineHeight = 18.sp,
        )
    }
}

/** "e330" → "E330", "e150d" → "E150d". OFF tags are lowercase. */
private fun formatAdditives(additives: List<String>): String =
    additives.joinToString(", ") {
        if (it.startsWith("e") && it.length > 1 && it[1].isDigit()) {
            "E" + it.substring(1)
        } else {
            it
        }
    }

private fun novaLabel(group: Int): String = when (group) {
    1 -> "unprocessed"
    2 -> "processed ingredients"
    3 -> "processed"
    4 -> "ultra-processed"
    else -> "unknown"
}

/** Detail-screen mg formatter — integer above 10 mg, one decimal otherwise. */
private fun formatMgForDetail(v: Double): String =
    if (v >= 10.0) "${v.toInt()} mg" else "%.1f mg".format(v)

/** Detail-screen microgram formatter for vitamin D etc. */
private fun formatMcgForDetail(v: Double): String =
    if (v >= 10.0) "${v.toInt()} mcg" else "%.1f mcg".format(v)

@Composable
private fun MacroRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
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
 * Persist a `food.eaten` event with the channel shape FoodScreen's
 * aggregator reads — see `FoodScreen.aggregateMacros`.
 *
 * Channels:
 *   - `name`       : Text — display label (also surfaces in the Recent list)
 *   - `grams`      : Real — amount eaten in grams
 *   - `kcal`       : Real — calories at the chosen amount
 *   - `carbs_g`    : Real — carbohydrates in grams
 *   - `protein_g`  : Real — protein in grams
 *   - `fat_g`      : Real — fat in grams
 *   - `sugar_g`    : Real — sugar in grams
 */
private fun logFood(
    item: FoodItem,
    grams: Double,
    macros: ResolvedMacros,
): PutEventOutcome {
    val now = System.currentTimeMillis()
    val correlationId = newCorrelationId()
    val parent = EventInput(
        timestampMs = now,
        eventType = "food.eaten",
        channels = parentFoodChannels(item, grams, correlationId),
        notes = item.brand?.let { "brand: $it" },
        topLevel = true,
    )
    return commitMeal(parent, item, macros, correlationId, now)
}

/**
 * Persist a meal's parent event plus all of its intake + composition children
 * in a single atomic [StorageRepository.putEvents] call — one network RPC, and
 * all-or-nothing so a meal never half-lands. The parent is element 0 of the
 * input list, so the first returned outcome is the parent's; that outcome
 * decides the Committed / Pending / Error result, matching the old behavior
 * where the children were fire-and-forget after a successful parent write.
 */
private fun commitMeal(
    parent: EventInput,
    item: FoodItem,
    macros: ResolvedMacros,
    correlationId: String,
    now: Long,
): PutEventOutcome {
    val events = buildList {
        add(parent)
        addAll(intakeChildren(macros, correlationId, now))
        addAll(compositionChildren(item, correlationId, now))
    }
    val outcomes = StorageRepository.putEvents(events, atomic = true).getOrElse { e ->
        return PutEventOutcome.Error(
            code = "INTERNAL",
            message = e.message ?: e::class.simpleName.orEmpty(),
        )
    }
    // putEvents returns outcomes in input order, so index 0 is the parent.
    return outcomes.firstOrNull() ?: PutEventOutcome.Error(
        code = "INTERNAL",
        message = "putEvents returned no outcomes",
    )
}

/**
 * Persist a `food.consumption_started` event (bug #5).
 *
 * Same channel shape as `food.eaten` so the "Today" totals can be derived
 * from the started/finished pair after `food.consumption_finished` lands.
 * Additional `correlation_id` text channel ties the two events together.
 */
private fun logFoodStarted(
    item: FoodItem,
    grams: Double,
    macros: ResolvedMacros,
    correlationId: String,
): PutEventOutcome {
    val now = System.currentTimeMillis()
    val parent = EventInput(
        timestampMs = now,
        eventType = "food.consumption_started",
        channels = parentFoodChannels(item, grams, correlationId),
        notes = item.brand?.let { "brand: $it" },
        topLevel = true,
    )
    return commitMeal(parent, item, macros, correlationId, now)
}

/**
 * Channels for the **parent** food.eaten / food.consumption_started event.
 *
 * Carries identity + composition only — name, grams, OFF tags, ingredients,
 * NOVA / Nutri-Score. Each nutrient (kcal, carbs, fat, caffeine, …) goes
 * out as its own `intake.<key>` child event (see [intakeChildren])
 * keyed by [correlationId], so "carbs over time" can scan a single event
 * type. `eco_score` is omitted on purpose — the user doesn't track it.
 */
private fun parentFoodChannels(
    item: FoodItem,
    grams: Double,
    correlationId: String,
): List<EventChannelInput> {
    val out = mutableListOf(
        EventChannelInput("name", OhdScalar.Text(item.name)),
        EventChannelInput("grams", OhdScalar.Real(grams)),
        EventChannelInput("correlation_id", OhdScalar.Text(correlationId)),
    )
    item.additives.takeIf { it.isNotEmpty() }?.let {
        out += EventChannelInput("additives", OhdScalar.Text(it.joinToString(",")))
    }
    item.allergens.takeIf { it.isNotEmpty() }?.let {
        out += EventChannelInput("allergens", OhdScalar.Text(it.joinToString(",")))
    }
    item.traces.takeIf { it.isNotEmpty() }?.let {
        out += EventChannelInput("traces", OhdScalar.Text(it.joinToString(",")))
    }
    item.ingredientsAnalysis.takeIf { it.isNotEmpty() }?.let {
        out += EventChannelInput("ingredients_analysis", OhdScalar.Text(it.joinToString(",")))
    }
    item.labels.takeIf { it.isNotEmpty() }?.let {
        out += EventChannelInput("labels", OhdScalar.Text(it.joinToString(",")))
    }
    item.ingredients.takeIf { it.isNotEmpty() }?.let {
        out += EventChannelInput("ingredients", OhdScalar.Text(it.joinToString(",")))
    }
    item.novaGroup?.let {
        out += EventChannelInput("nova_group", OhdScalar.Int(it.toLong()))
    }
    item.nutriScore?.let {
        out += EventChannelInput("nutri_score", OhdScalar.Text(it))
    }
    return out
}

/**
 * Build one `intake.<nutrient>` event per non-zero nutrient resolved for
 * this serving. Children are flagged `topLevel = false` so they don't
 * clutter Recent / History but search queries that target a specific
 * intake type still find them. Pre-registered types live in migration 018;
 * novel nutriments auto-register via dynamic channel registration.
 *
 * Returns the events instead of writing them — the caller folds them into the
 * single atomic [StorageRepository.putEvents] batch for the meal.
 */
private fun intakeChildren(
    macros: ResolvedMacros,
    correlationId: String,
    timestampMs: Long,
): List<EventInput> = buildList {
    fun add(eventType: String, value: Double, unit: String) {
        if (value <= 0.0) return
        add(
            EventInput(
                timestampMs = timestampMs,
                eventType = eventType,
                channels = listOf(
                    EventChannelInput("value", OhdScalar.Real(value)),
                    EventChannelInput("unit", OhdScalar.Text(unit)),
                    EventChannelInput("correlation_id", OhdScalar.Text(correlationId)),
                ),
                topLevel = false,
            ),
        )
    }
    add("intake.kcal", macros.kcal.toDouble(), "kcal")
    add("intake.carbs_g", macros.carbsG, "g")
    add("intake.protein_g", macros.proteinG, "g")
    add("intake.fat_g", macros.fatG, "g")
    add("intake.sugar_g", macros.sugarG, "g")
    add("intake.fiber_g", macros.fiberG, "g")
    add("intake.saturated_fat_g", macros.saturatedFatG, "g")
    add("intake.trans_fat_g", macros.transFatG, "g")
    add("intake.sodium_mg", macros.sodiumMg, "mg")
    add("intake.cholesterol_mg", macros.cholesterolMg, "mg")
    add("intake.potassium_mg", macros.potassiumMg, "mg")
    add("intake.calcium_mg", macros.calciumMg, "mg")
    add("intake.iron_mg", macros.ironMg, "mg")
    add("intake.vitamin_c_mg", macros.vitaminCMg, "mg")
    add("intake.vitamin_d_mcg", macros.vitaminDMcg, "mcg")
    add("intake.caffeine_mg", macros.caffeineMg, "mg")
}

/**
 * Build one child event per composition tag — allergens, traces, additives,
 * labels, ingredients, ingredient-analysis (vegan / palm-oil-free / …).
 *
 * Pattern: `event_type = composition.<category>.<slug>`. Each event carries
 * the parent's `correlation_id` and `top_level = false`. With this shape,
 * "when did I eat gluten" is a one-line type filter
 * (`composition.allergen.gluten`) — no JSON parsing, no scanning channel
 * lists. Dynamic channel registration handles the long tail of slugs as
 * they appear.
 *
 * Returns the events instead of writing them — the caller folds them into the
 * single atomic [StorageRepository.putEvents] batch for the meal.
 */
private fun compositionChildren(
    item: FoodItem,
    correlationId: String,
    timestampMs: Long,
): List<EventInput> = buildList {
    fun add(category: String, slug: String) {
        if (slug.isBlank()) return
        val safeSlug = slug.lowercase()
            .replace(Regex("[^a-z0-9_]"), "_")
            .trim('_')
            .takeIf { it.isNotEmpty() } ?: return
        add(
            EventInput(
                timestampMs = timestampMs,
                eventType = "composition.$category.$safeSlug",
                channels = listOf(
                    EventChannelInput("correlation_id", OhdScalar.Text(correlationId)),
                ),
                topLevel = false,
            ),
        )
    }
    item.allergens.forEach { add("allergen", it) }
    item.traces.forEach { add("trace", it) }
    item.additives.forEach { add("additive", it) }
    item.labels.forEach { add("label", it) }
    item.ingredients.forEach { add("ingredient", it) }
    item.ingredientsAnalysis.forEach { add("analysis", it) }
}

/**
 * Generate a fresh correlation ID for a consumption_started/finished pair.
 *
 * Not a real ULID — the storage core mints those on the event itself. This
 * is a 26-character Crockford-base32-shaped string built from the current
 * timestamp + random bytes; collision probability is negligible for the
 * "open red bull at 4 PM" use case.
 */
private fun newCorrelationId(): String {
    val nowMs = System.currentTimeMillis()
    val rand = java.util.UUID.randomUUID()
    return "FCID-${nowMs.toString(36)}-${rand.toString().take(8)}"
}
