package com.ohd.connect.ui.screens.settings

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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdToggle
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Food & Nutrition settings — Pencil `VCokI` "Food & Nutrition" panel.
 *
 * Three cards stacked:
 *  1. **Daily targets** — kcal / carbs / protein / fat numeric inputs that
 *     persist back into Auth on every keystroke. The FoodScreen v1 still
 *     hard-codes its `OhdNutriGauge` constants; once the food agent wires
 *     it, the gauge will read these prefs at composition time.
 *  2. **Barcode lookups** — toggle for OpenFoodFacts. v1 only flips the
 *     pref; no network call is shipped.
 *  3. **Recent foods** — read-only lifetime count of `food.eaten` events
 *     from storage. Skips silently if storage isn't open yet (Setup
 *     incomplete) — the row shows "—".
 */
@Composable
fun FoodSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current

    // Targets — hoist into mutableState so the four OhdFields stay in sync
    // and every commit also persists.
    val initial = remember { Auth.loadFoodTargets(ctx) }
    var kcal by remember { mutableStateOf(initial.kcal.toString()) }
    var carbs by remember { mutableStateOf(initial.carbsG.toString()) }
    var protein by remember { mutableStateOf(initial.proteinG.toString()) }
    var fat by remember { mutableStateOf(initial.fatG.toString()) }

    // OpenFoodFacts toggle.
    var offEnabled by remember { mutableStateOf(Auth.openFoodFactsEnabled(ctx)) }

    // Lifetime food.eaten count. `null` while we're still computing or if
    // the storage handle isn't open. The query is cheap (COUNT(*) under
    // self-session) so we just run it on the composition's first frame.
    var foodCount by remember { mutableStateOf<Long?>(null) }
    LaunchedEffect(Unit) {
        foodCount = StorageRepository
            .countEvents(EventFilter(eventTypesIn = listOf("food.eaten"), limit = null))
            .getOrNull()
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Food & Nutrition", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // ---- 1. Daily targets ------------------------------------------
            OhdCard(title = "Daily targets") {
                Text(
                    text = "Used by the home dashboard's nutrition gauge. Leave a field empty to fall back to the default.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
                OhdField(
                    label = "Energy (kcal)",
                    value = kcal,
                    onValueChange = { v ->
                        kcal = v.filter { it.isDigit() }.take(5)
                        persistTargets(ctx, kcal, carbs, protein, fat)
                    },
                    placeholder = "2000",
                    keyboardType = KeyboardType.Number,
                )
                OhdField(
                    label = "Carbs (g)",
                    value = carbs,
                    onValueChange = { v ->
                        carbs = v.filter { it.isDigit() }.take(4)
                        persistTargets(ctx, kcal, carbs, protein, fat)
                    },
                    placeholder = "250",
                    keyboardType = KeyboardType.Number,
                )
                OhdField(
                    label = "Protein (g)",
                    value = protein,
                    onValueChange = { v ->
                        protein = v.filter { it.isDigit() }.take(4)
                        persistTargets(ctx, kcal, carbs, protein, fat)
                    },
                    placeholder = "75",
                    keyboardType = KeyboardType.Number,
                )
                OhdField(
                    label = "Fat (g)",
                    value = fat,
                    onValueChange = { v ->
                        fat = v.filter { it.isDigit() }.take(4)
                        persistTargets(ctx, kcal, carbs, protein, fat)
                    },
                    placeholder = "65",
                    keyboardType = KeyboardType.Number,
                )
            }

            // ---- 2. Barcode lookups ----------------------------------------
            OhdCard(title = "Barcode lookups") {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = "Allow lookups via OpenFoodFacts",
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W500,
                            fontSize = 14.sp,
                            color = OhdColors.Ink,
                        )
                        Text(
                            text = "When on, scanning a barcode queries OpenFoodFacts for macros. Off by default — no network call is made.",
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W400,
                            fontSize = 12.sp,
                            lineHeight = 18.sp,
                            color = OhdColors.Muted,
                        )
                    }
                    OhdToggle(
                        checked = offEnabled,
                        onCheckedChange = { v ->
                            offEnabled = v
                            Auth.setOpenFoodFactsEnabled(ctx, v)
                        },
                    )
                }
            }

            // ---- 3. Recent foods -------------------------------------------
            OhdCard(title = "Recent foods") {
                val text = when (foodCount) {
                    null -> "—"
                    else -> "${foodCount} logged this lifetime"
                }
                Text(
                    text = text,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
            }
        }
    }
}

/**
 * Commit the four target fields back to Auth. Called on every keystroke; the
 * shared-prefs write is cheap (single `apply()`) and the user wipes data
 * between installs anyway, so we don't bother with debouncing.
 *
 * Empty / unparseable values fall back to the defaults baked into
 * `Auth.FoodTargets`.
 */
private fun persistTargets(
    ctx: android.content.Context,
    kcal: String,
    carbs: String,
    protein: String,
    fat: String,
) {
    val defaults = Auth.FoodTargets()
    Auth.saveFoodTargets(
        ctx,
        Auth.FoodTargets(
            kcal = kcal.toIntOrNull() ?: defaults.kcal,
            carbsG = carbs.toIntOrNull() ?: defaults.carbsG,
            proteinG = protein.toIntOrNull() ?: defaults.proteinG,
            fatG = fat.toIntOrNull() ?: defaults.fatG,
        ),
    )
}
