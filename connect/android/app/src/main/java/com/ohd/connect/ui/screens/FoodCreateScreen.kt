package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.CustomFoodStore
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Create a user-defined food item.
 *
 * Reached from [FoodSearchScreen]'s "+ Create custom food" affordance when
 * neither the in-app dictionary nor OpenFoodFacts has what the user is
 * trying to log. The user types in macros per 100 g; [CustomFoodStore]
 * persists the row so subsequent searches surface it.
 *
 * The form mirrors the full [FoodItem] / [NutritionFacts] / [Serving]
 * shape — Basics + headline macros are always visible, the rest live
 * behind small expanders so the screen isn't a wall of inputs:
 *
 *  1. Basics (name / brand / description) — always visible.
 *  2. Per-100 g nutrition — five headline rows always visible, an
 *     expander reveals fiber, sat-fat, trans-fat, sodium, cholesterol,
 *     potassium, calcium, iron, vitamin C/D, caffeine.
 *  3. Serving sizes — optional package + portion serving (label + g).
 *  4. Allergens & traces — multi-select chip rows on the OFF token list.
 *  5. Ingredients & additives — comma-separated text areas + analysis
 *     and label chip rows.
 *  6. Scores — NOVA 1-4, Nutri-Score A-E, Eco-Score A-E pill rows.
 *
 * On save we run the write off-main (mirroring [FoodDetailScreen]) then
 * pop back to the search screen. The search re-queries [CustomFoodStore]
 * on recomposition so the new row shows up automatically.
 */
@Composable
fun FoodCreateScreen(
    prefill: String?,
    onBack: () -> Unit,
    onSaved: (summary: String) -> Unit,
    onError: (message: String) -> Unit = {},
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    // ---- Basics ----------------------------------------------------------
    //
    // Prefill source-router: a numeric prefill (8-13 digits) is an
    // EAN/UPC/GTIN — the user scanned a barcode and OFF returned nothing,
    // so they're filling in the missing product. Route it to the
    // dedicated `barcode` field instead of dumping a 13-digit number into
    // the food's display name. Non-numeric prefill stays in `name` (the
    // user typed a search term and the dictionary / OFF didn't have it).
    val prefillIsBarcode = remember(prefill) {
        prefill != null && Regex("^\\d{8,13}$").matches(prefill.trim())
    }
    var name by remember {
        mutableStateOf(if (prefillIsBarcode) "" else prefill.orEmpty())
    }
    var brand by remember { mutableStateOf("") }
    var barcode by remember {
        mutableStateOf(if (prefillIsBarcode) prefill!!.trim() else "")
    }
    var description by remember { mutableStateOf("") }

    // ---- Per-100 g headline macros --------------------------------------
    var kcal by remember { mutableStateOf("") }
    var carbs by remember { mutableStateOf("") }
    var protein by remember { mutableStateOf("") }
    var fat by remember { mutableStateOf("") }
    var sugar by remember { mutableStateOf("") }

    // ---- Per-100 g extended micronutrients ------------------------------
    var fiber by remember { mutableStateOf("") }
    var saturatedFat by remember { mutableStateOf("") }
    var transFat by remember { mutableStateOf("") }
    var sodium by remember { mutableStateOf("") }
    var cholesterol by remember { mutableStateOf("") }
    var potassium by remember { mutableStateOf("") }
    var calcium by remember { mutableStateOf("") }
    var iron by remember { mutableStateOf("") }
    var vitaminC by remember { mutableStateOf("") }
    var vitaminD by remember { mutableStateOf("") }
    var caffeine by remember { mutableStateOf("") }

    // ---- Serving sizes --------------------------------------------------
    var packageLabel by remember { mutableStateOf("") }
    var packageGrams by remember { mutableStateOf("") }
    var portionLabel by remember { mutableStateOf("") }
    var portionGrams by remember { mutableStateOf("") }

    // ---- Allergens & traces (OFF token list) ----------------------------
    var allergens by remember { mutableStateOf(setOf<String>()) }
    var traces by remember { mutableStateOf(setOf<String>()) }

    // ---- Ingredients / additives / analysis / labels --------------------
    var ingredients by remember { mutableStateOf("") }
    var additives by remember { mutableStateOf("") }
    var analysis by remember { mutableStateOf(setOf<String>()) }
    var labels by remember { mutableStateOf(setOf<String>()) }

    // ---- Scores ---------------------------------------------------------
    var novaGroup by remember { mutableStateOf<Int?>(null) }
    var nutriScore by remember { mutableStateOf<String?>(null) }
    var ecoScore by remember { mutableStateOf<String?>(null) }

    // ---- Expander state (local — not persisted) -------------------------
    var moreNutrientsOpen by remember { mutableStateOf(false) }
    var servingsOpen by remember { mutableStateOf(false) }
    var allergensOpen by remember { mutableStateOf(false) }
    var ingredientsOpen by remember { mutableStateOf(false) }
    var scoresOpen by remember { mutableStateOf(false) }

    var submitting by remember { mutableStateOf(false) }

    val canSave = name.trim().isNotEmpty() && !submitting

    val onSave: () -> Unit = save@{
        if (!canSave) return@save
        val trimmedName = name.trim()
        val trimmedBrand = brand.trim().ifEmpty { null }
        val trimmedDescription = description.trim().ifEmpty {
            // FoodDetailScreen renders the description paragraph
            // unconditionally; give it a neutral fallback so the row doesn't
            // look broken when the user skips the optional field.
            "User-created food."
        }

        val per100g = NutritionFacts(
            kcal = kcal.trim().toIntOrNull() ?: 0,
            carbsG = carbs.trim().toDoubleOrNull() ?: 0.0,
            proteinG = protein.trim().toDoubleOrNull() ?: 0.0,
            fatG = fat.trim().toDoubleOrNull() ?: 0.0,
            sugarG = sugar.trim().toDoubleOrNull() ?: 0.0,
            fiberG = fiber.trim().toDoubleOrNull() ?: 0.0,
            saturatedFatG = saturatedFat.trim().toDoubleOrNull() ?: 0.0,
            transFatG = transFat.trim().toDoubleOrNull() ?: 0.0,
            sodiumMg = sodium.trim().toDoubleOrNull() ?: 0.0,
            cholesterolMg = cholesterol.trim().toDoubleOrNull() ?: 0.0,
            potassiumMg = potassium.trim().toDoubleOrNull() ?: 0.0,
            calciumMg = calcium.trim().toDoubleOrNull() ?: 0.0,
            ironMg = iron.trim().toDoubleOrNull() ?: 0.0,
            vitaminCMg = vitaminC.trim().toDoubleOrNull() ?: 0.0,
            vitaminDMcg = vitaminD.trim().toDoubleOrNull() ?: 0.0,
            caffeineMg = caffeine.trim().toDoubleOrNull() ?: 0.0,
        )

        val pkgServing = buildServing(packageLabel, packageGrams)
        val defServing = buildServing(portionLabel, portionGrams)

        val food = FoodItem(
            name = trimmedName,
            brand = trimmedBrand,
            barcode = barcode.trim().ifEmpty { null },
            source = "user-created",
            description = trimmedDescription,
            per100g = per100g,
            packageServing = pkgServing,
            defaultPortion = defServing,
            additives = splitCsv(additives),
            allergens = allergens.toList(),
            traces = traces.toList(),
            ingredients = splitCsv(ingredients),
            ingredientsAnalysis = analysis.toList(),
            labels = labels.toList(),
            novaGroup = novaGroup,
            nutriScore = nutriScore,
            ecoScore = ecoScore,
        )

        submitting = true
        scope.launch(Dispatchers.IO) {
            val result = runCatching { CustomFoodStore.add(ctx, food) }
            withContext(Dispatchers.Main) {
                submitting = false
                result
                    .onSuccess {
                        onSaved("Saved \"$trimmedName\"")
                        onBack()
                    }
                    .onFailure { e ->
                        onError("Couldn't save: ${e.message ?: e::class.simpleName.orEmpty()}")
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
        OhdTopBar(title = "Create food", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // ===== 1. BASICS (always visible) ============================
            OhdField(
                label = "Name",
                value = name,
                onValueChange = { name = it },
                placeholder = "Homemade granola",
            )
            OhdField(
                label = "Brand (optional)",
                value = brand,
                onValueChange = { brand = it },
                placeholder = "—",
            )
            OhdField(
                label = "Barcode (optional)",
                value = barcode,
                onValueChange = { input ->
                    // Keep only digits — EAN / UPC / GTIN. Cap at 14 (the
                    // longest GTIN form) so a paste doesn't run away.
                    barcode = input.filter { it.isDigit() }.take(14)
                },
                placeholder = "EAN / UPC — e.g. 5901234567890",
            )
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    text = "Description (optional)",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
                OhdInput(
                    value = description,
                    onValueChange = { description = it },
                    placeholder = "Recipe / source / notes…",
                    singleLine = false,
                )
            }

            // ===== 2. PER-100 G NUTRITION ================================
            OhdSectionHeaderInline(text = "PER 100 G")

            // Calories — integer kcal.
            OhdField(
                label = "Calories (kcal)",
                value = kcal,
                onValueChange = { kcal = it.filter { ch -> ch.isDigit() } },
                placeholder = "0",
                keyboardType = KeyboardType.Number,
            )

            // Macros — grams, accept decimal entry.
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OhdField(
                    label = "Carbs (g)",
                    value = carbs,
                    onValueChange = { carbs = sanitizeDecimal(it) },
                    placeholder = "0",
                    keyboardType = KeyboardType.Decimal,
                    modifier = Modifier.weight(1f),
                )
                OhdField(
                    label = "Protein (g)",
                    value = protein,
                    onValueChange = { protein = sanitizeDecimal(it) },
                    placeholder = "0",
                    keyboardType = KeyboardType.Decimal,
                    modifier = Modifier.weight(1f),
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OhdField(
                    label = "Fat (g)",
                    value = fat,
                    onValueChange = { fat = sanitizeDecimal(it) },
                    placeholder = "0",
                    keyboardType = KeyboardType.Decimal,
                    modifier = Modifier.weight(1f),
                )
                OhdField(
                    label = "Sugar (g)",
                    value = sugar,
                    onValueChange = { sugar = sanitizeDecimal(it) },
                    placeholder = "0",
                    keyboardType = KeyboardType.Decimal,
                    modifier = Modifier.weight(1f),
                )
            }

            ExpanderHeader(
                label = "Show more nutrients",
                expanded = moreNutrientsOpen,
                onToggle = { moreNutrientsOpen = !moreNutrientsOpen },
            )
            if (moreNutrientsOpen) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OhdField(
                        label = "Fiber (g)",
                        value = fiber,
                        onValueChange = { fiber = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                    OhdField(
                        label = "Saturated fat (g)",
                        value = saturatedFat,
                        onValueChange = { saturatedFat = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OhdField(
                        label = "Trans fat (g)",
                        value = transFat,
                        onValueChange = { transFat = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                    OhdField(
                        label = "Sodium (mg)",
                        value = sodium,
                        onValueChange = { sodium = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OhdField(
                        label = "Cholesterol (mg)",
                        value = cholesterol,
                        onValueChange = { cholesterol = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                    OhdField(
                        label = "Potassium (mg)",
                        value = potassium,
                        onValueChange = { potassium = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OhdField(
                        label = "Calcium (mg)",
                        value = calcium,
                        onValueChange = { calcium = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                    OhdField(
                        label = "Iron (mg)",
                        value = iron,
                        onValueChange = { iron = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OhdField(
                        label = "Vitamin C (mg)",
                        value = vitaminC,
                        onValueChange = { vitaminC = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                    OhdField(
                        label = "Vitamin D (µg)",
                        value = vitaminD,
                        onValueChange = { vitaminD = sanitizeDecimal(it) },
                        placeholder = "0",
                        keyboardType = KeyboardType.Decimal,
                        modifier = Modifier.weight(1f),
                    )
                }
                OhdField(
                    label = "Caffeine (mg)",
                    value = caffeine,
                    onValueChange = { caffeine = sanitizeDecimal(it) },
                    placeholder = "0",
                    keyboardType = KeyboardType.Decimal,
                )
            }

            // ===== 3. SERVING SIZES ======================================
            ExpanderHeader(
                label = "Serving sizes",
                expanded = servingsOpen,
                onToggle = { servingsOpen = !servingsOpen },
            )
            if (servingsOpen) {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = "Package serving",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 12.sp,
                        color = OhdColors.Muted,
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        OhdField(
                            label = "Label",
                            value = packageLabel,
                            onValueChange = { packageLabel = it },
                            placeholder = "Bottle",
                            modifier = Modifier.weight(1.4f),
                        )
                        OhdField(
                            label = "Grams",
                            value = packageGrams,
                            onValueChange = { packageGrams = sanitizeDecimal(it) },
                            placeholder = "0",
                            keyboardType = KeyboardType.Decimal,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = "Default portion",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 12.sp,
                        color = OhdColors.Muted,
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        OhdField(
                            label = "Label",
                            value = portionLabel,
                            onValueChange = { portionLabel = it },
                            placeholder = "Bowl",
                            modifier = Modifier.weight(1.4f),
                        )
                        OhdField(
                            label = "Grams",
                            value = portionGrams,
                            onValueChange = { portionGrams = sanitizeDecimal(it) },
                            placeholder = "0",
                            keyboardType = KeyboardType.Decimal,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }

            // ===== 4. ALLERGENS & TRACES =================================
            ExpanderHeader(
                label = "Allergens",
                expanded = allergensOpen,
                onToggle = { allergensOpen = !allergensOpen },
            )
            if (allergensOpen) {
                FormSubLabel("Contains")
                ChipRow(
                    options = OFF_ALLERGENS,
                    selected = allergens,
                    onToggle = { tok ->
                        allergens = if (tok in allergens) allergens - tok else allergens + tok
                    },
                )
                FormSubLabel("May contain (traces)")
                ChipRow(
                    options = OFF_ALLERGENS,
                    selected = traces,
                    onToggle = { tok ->
                        traces = if (tok in traces) traces - tok else traces + tok
                    },
                )
            }

            // ===== 5. INGREDIENTS, ADDITIVES, ANALYSIS, LABELS ===========
            ExpanderHeader(
                label = "Ingredients",
                expanded = ingredientsOpen,
                onToggle = { ingredientsOpen = !ingredientsOpen },
            )
            if (ingredientsOpen) {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = "Ingredients (comma-separated)",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                    )
                    OhdInput(
                        value = ingredients,
                        onValueChange = { ingredients = it },
                        placeholder = "oats, honey, almonds…",
                        singleLine = false,
                    )
                }
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = "Additives (comma-separated, e.g. e330, e150d)",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                    )
                    OhdInput(
                        value = additives,
                        onValueChange = { additives = it },
                        placeholder = "e330, e150d…",
                        singleLine = false,
                    )
                }
                FormSubLabel("Ingredient analysis")
                ChipRow(
                    options = OFF_ANALYSIS,
                    selected = analysis,
                    onToggle = { tok ->
                        analysis = if (tok in analysis) analysis - tok else analysis + tok
                    },
                )
                FormSubLabel("Labels")
                ChipRow(
                    options = OFF_LABELS,
                    selected = labels,
                    onToggle = { tok ->
                        labels = if (tok in labels) labels - tok else labels + tok
                    },
                )
            }

            // ===== 6. SCORES =============================================
            ExpanderHeader(
                label = "Scores",
                expanded = scoresOpen,
                onToggle = { scoresOpen = !scoresOpen },
            )
            if (scoresOpen) {
                FormSubLabel("NOVA group")
                PillRow(
                    options = listOf("1", "2", "3", "4"),
                    selectedLabel = novaGroup?.toString(),
                    unselectedLabel = "Unclassified",
                    onSelect = { picked ->
                        novaGroup = if (novaGroup?.toString() == picked) null else picked.toIntOrNull()
                    },
                )
                FormSubLabel("Nutri-Score")
                PillRow(
                    options = listOf("A", "B", "C", "D", "E"),
                    selectedLabel = nutriScore?.uppercase(),
                    onSelect = { picked ->
                        val low = picked.lowercase()
                        nutriScore = if (nutriScore == low) null else low
                    },
                )
                FormSubLabel("Eco-Score")
                PillRow(
                    options = listOf("A", "B", "C", "D", "E"),
                    selectedLabel = ecoScore?.uppercase(),
                    onSelect = { picked ->
                        val low = picked.lowercase()
                        ecoScore = if (ecoScore == low) null else low
                    },
                )
            }

            // ===== Save ==================================================
            Box(modifier = Modifier.height(4.dp))
            OhdButton(
                label = if (submitting) "Saving…" else "Save",
                onClick = onSave,
                enabled = canSave,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

// =============================================================================
// Local form primitives — section expander, chip row, pill row.
// Kept private to this file: lifting to /ui/components would over-generalise
// for a single-screen use case; the chip/pill shapes deliberately mirror the
// existing FilterChip/AmountChip styling without the long-click / amount logic.
// =============================================================================

/**
 * Section expander. Renders an OhdSectionHeader-style row with a ▾/▸
 * indicator on the right. Clicking anywhere on the row toggles
 * [expanded].
 */
@Composable
private fun ExpanderHeader(
    label: String,
    expanded: Boolean,
    onToggle: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onToggle() }
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = label.uppercase(),
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 11.sp,
            letterSpacing = 2.sp,
            color = OhdColors.Muted,
        )
        Text(
            text = if (expanded) "▾" else "▸",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
    }
}

/**
 * A tight column-level sub-label, used inside expanders to title a chip
 * row ("Contains", "May contain (traces)", …). Smaller than a full
 * section header — the expander is already that — but still visually
 * distinct from a field label.
 */
@Composable
private fun FormSubLabel(text: String) {
    Text(
        text = text,
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 12.sp,
        color = OhdColors.Muted,
    )
}

/**
 * Inline section header — same typography as OhdSectionHeader but
 * without the standalone `[h=16]` padding so it lines up with the
 * surrounding column inset.
 */
@Composable
private fun OhdSectionHeaderInline(text: String) {
    Text(
        text = text.uppercase(),
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 11.sp,
        letterSpacing = 2.sp,
        color = OhdColors.Muted,
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
    )
}

/**
 * Horizontally-scrollable row of toggleable filter chips. Mirrors the
 * FilterChip in RecentEventsScreen — rounded pill, Ink fill when
 * selected, BgElevated otherwise. Tokens are shown title-cased
 * (`Gluten`) but the underlying selection set stores the lowercase OFF
 * token (`gluten`).
 */
@Composable
private fun ChipRow(
    options: List<String>,
    selected: Set<String>,
    onToggle: (String) -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        options.forEach { token ->
            ToggleChip(
                label = token.replace('-', ' ').replaceFirstChar { it.uppercase() },
                selected = token in selected,
                onClick = { onToggle(token) },
            )
        }
    }
}

@Composable
private fun ToggleChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(16.dp)
    val bg = if (selected) OhdColors.Ink else OhdColors.BgElevated
    val fg = if (selected) OhdColors.White else OhdColors.Muted
    Box(
        modifier = Modifier
            .height(30.dp)
            .background(bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .clickable { onClick() }
            .padding(horizontal = 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = if (selected) FontWeight.W500 else FontWeight.W400,
            fontSize = 12.sp,
            color = fg,
        )
    }
}

/**
 * Row of single-select pill buttons (NOVA / Nutri-Score / Eco-Score).
 * Tap a selected pill again to clear back to "unclassified". The
 * unselected display label (e.g. "Unclassified") is shown to the
 * left when no pill is active.
 */
@Composable
private fun PillRow(
    options: List<String>,
    selectedLabel: String?,
    unselectedLabel: String = "None",
    onSelect: (String) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = if (selectedLabel == null) unselectedLabel else "Selected:",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
        options.forEach { opt ->
            ToggleChip(
                label = opt,
                selected = selectedLabel == opt,
                onClick = { onSelect(opt) },
            )
        }
    }
}

// =============================================================================
// OFF token lists — kept in one place so the chip rows above and the
// CustomFoodStore serializer share a single source of truth.
// Token forms are the un-prefixed OFF tags (`milk`, not `en:milk`),
// matching what FoodItem.allergens / .labels / .ingredientsAnalysis carry.
// =============================================================================

/**
 * Common OFF allergen tokens. Stable subset of the canonical OFF list
 * (https://world.openfoodfacts.org/allergens); enough to cover the EU-14
 * mandatory disclosure set without overwhelming the chip row.
 */
private val OFF_ALLERGENS: List<String> = listOf(
    "gluten",
    "milk",
    "eggs",
    "nuts",
    "peanuts",
    "soy",
    "fish",
    "crustaceans",
    "sesame",
    "celery",
    "mustard",
    "sulphites",
    "lupin",
    "molluscs",
)

/** OFF ingredient-analysis tags surfaced as user-toggleable. */
private val OFF_ANALYSIS: List<String> = listOf(
    "vegan",
    "vegetarian",
    "palm-oil-free",
    "contains-palm-oil",
)

/** OFF labels surfaced as user-toggleable. */
private val OFF_LABELS: List<String> = listOf(
    "organic",
    "bio",
    "fairtrade",
    "gluten-free",
    "kosher",
    "halal",
    "vegan",
    "vegetarian",
)

// =============================================================================
// Helpers
// =============================================================================

/**
 * Build a Serving from a label + grams pair. Returns null when either
 * is blank or the grams aren't a positive number — the form's "leave
 * blank to skip" semantic.
 */
private fun buildServing(label: String, grams: String): Serving? {
    val trimmedLabel = label.trim()
    val parsedGrams = grams.trim().toDoubleOrNull()
    if (trimmedLabel.isEmpty() || parsedGrams == null || parsedGrams <= 0.0) return null
    return Serving(name = trimmedLabel, grams = parsedGrams)
}

/**
 * Split a comma-separated ingredient/additive list into a clean list.
 * Trims each entry, drops blanks, caps at 50 to mirror the FoodItem.ingredients
 * comment about OFF's 50-entry hard ceiling.
 */
private fun splitCsv(raw: String): List<String> =
    raw.split(',')
        .map { it.trim() }
        .filter { it.isNotEmpty() }
        .take(50)

/**
 * Allow digits + a single decimal separator (locale-friendly: accept either
 * `.` or `,` from the soft keyboard — the parser coerces to `Double` later).
 * Strips anything else so the [KeyboardType.Decimal] keyboard stays the only
 * legal input surface.
 */
private fun sanitizeDecimal(raw: String): String {
    val normalised = raw.replace(',', '.')
    var seenDot = false
    val out = StringBuilder()
    for (ch in normalised) {
        when {
            ch.isDigit() -> out.append(ch)
            ch == '.' && !seenDot -> {
                seenDot = true
                out.append(ch)
            }
        }
    }
    return out.toString()
}
