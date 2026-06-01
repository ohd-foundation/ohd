package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.CustomFoodStore
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
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
 * On save we run the write off-main (mirroring the [FoodDetailScreen]
 * convention — even though `CustomFoodStore` is local-only and quick, we
 * stay on the main pattern so any future migration to a heavier backing
 * store stays behind a single boundary) then pop back to the search
 * screen. The search re-queries `CustomFoodStore` on recomposition so the
 * new row shows up automatically.
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

    var name by remember { mutableStateOf(prefill.orEmpty()) }
    var brand by remember { mutableStateOf("") }
    var kcal by remember { mutableStateOf("") }
    var carbs by remember { mutableStateOf("") }
    var protein by remember { mutableStateOf("") }
    var fat by remember { mutableStateOf("") }
    var sugar by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    var submitting by remember { mutableStateOf(false) }

    val canSave = name.trim().isNotEmpty() && !submitting

    val onSave: () -> Unit = save@{
        if (!canSave) return@save
        val trimmedName = name.trim()
        val parsedKcal = kcal.trim().toIntOrNull() ?: 0
        val parsedCarbs = carbs.trim().toDoubleOrNull() ?: 0.0
        val parsedProtein = protein.trim().toDoubleOrNull() ?: 0.0
        val parsedFat = fat.trim().toDoubleOrNull() ?: 0.0
        val parsedSugar = sugar.trim().toDoubleOrNull() ?: 0.0
        val trimmedBrand = brand.trim().ifEmpty { null }
        val trimmedDescription = description.trim().ifEmpty {
            // FoodDetailScreen renders the description paragraph
            // unconditionally; give it a neutral fallback so the row doesn't
            // look broken when the user skips the optional field.
            "User-created food."
        }

        val food = FoodItem(
            name = trimmedName,
            brand = trimmedBrand,
            source = "user-created",
            description = trimmedDescription,
            per100g = NutritionFacts(
                kcal = parsedKcal,
                carbsG = parsedCarbs,
                proteinG = parsedProtein,
                fatG = parsedFat,
                sugarG = parsedSugar,
            ),
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

            OhdSectionHeader(text = "PER 100 G")

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

            OhdSectionHeader(text = "DESCRIPTION (OPTIONAL)")
            OhdInput(
                value = description,
                onValueChange = { description = it },
                placeholder = "Recipe / source / notes…",
                singleLine = false,
            )

            OhdButton(
                label = if (submitting) "Saving…" else "Save",
                onClick = onSave,
                enabled = canSave,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

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
