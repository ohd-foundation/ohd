package com.ohd.connect.ui.screens.settings

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
import com.ohd.connect.data.Activity
import com.ohd.connect.data.Goal
import com.ohd.connect.data.NutritionGoalsStore
import com.ohd.connect.data.NutritionOverrides
import com.ohd.connect.data.NutritionProfile
import com.ohd.connect.data.NutritionTargets
import com.ohd.connect.data.Sex
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Nutrition goals — Settings → Nutrition goals.
 *
 * Three sections:
 *
 *   1. **Your profile** — biological inputs (sex / age / height / weight)
 *      + lifestyle (activity level) + balance (goal).
 *   2. **Recommended targets** — live preview computed from the *currently
 *      edited* profile via [NutritionGoalsStore.computedTargets]. When the
 *      profile is incomplete, shows a "fill in the rest" hint and the WHO
 *      fallback gets used until then.
 *   3. **Overrides** — five optional kcal/macro numeric fields that
 *      override the computed values per row. Blank = no override.
 *
 * Save runs off-main (mirrors [com.ohd.connect.ui.screens.FoodDetailScreen]),
 * then toasts "Saved" and pops back. The Food tab gauges call
 * [NutritionGoalsStore.effectiveTargets] on every recomposition, so the new
 * targets land without explicit invalidation.
 */
@Composable
fun NutritionGoalsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onToast: (String) -> Unit = {},
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    // Snapshot persisted state on first composition. The user can edit
    // freely; nothing is written until Save.
    val initialProfile = remember { NutritionGoalsStore.loadProfile(ctx) }
    val initialOverrides = remember { NutritionGoalsStore.loadOverrides(ctx) }

    // ---- Profile editor state ------------------------------------------
    var sex by remember { mutableStateOf(initialProfile.sex) }
    var age by remember { mutableStateOf(initialProfile.ageYears?.toString().orEmpty()) }
    var heightCm by remember {
        mutableStateOf(initialProfile.heightCm?.let { formatDecimal(it) }.orEmpty())
    }
    var weightKg by remember {
        mutableStateOf(initialProfile.weightKg?.let { formatDecimal(it) }.orEmpty())
    }
    var activity by remember { mutableStateOf(initialProfile.activity) }
    var goal by remember { mutableStateOf(initialProfile.goal) }

    // ---- Override editor state -----------------------------------------
    var overrideKcal by remember { mutableStateOf(initialOverrides.kcal?.toString().orEmpty()) }
    var overrideCarbs by remember { mutableStateOf(initialOverrides.carbsG?.toString().orEmpty()) }
    var overrideProtein by remember { mutableStateOf(initialOverrides.proteinG?.toString().orEmpty()) }
    var overrideFat by remember { mutableStateOf(initialOverrides.fatG?.toString().orEmpty()) }
    var overrideSugar by remember { mutableStateOf(initialOverrides.sugarG?.toString().orEmpty()) }

    var submitting by remember { mutableStateOf(false) }

    // Reconstruct the currently-edited profile each recomposition so the
    // "Recommended targets" preview stays live as the user types.
    val currentProfile = NutritionProfile(
        sex = sex,
        ageYears = age.toIntOrNull()?.takeIf { it > 0 },
        heightCm = heightCm.toDoubleOrNull()?.takeIf { it > 0.0 },
        weightKg = weightKg.toDoubleOrNull()?.takeIf { it > 0.0 },
        activity = activity,
        goal = goal,
    )
    val preview: NutritionTargets? = NutritionGoalsStore.computedTargets(currentProfile)

    val onSave: () -> Unit = save@{
        if (submitting) return@save
        submitting = true
        val toPersist = currentProfile
        val overrides = NutritionOverrides(
            kcal = overrideKcal.toIntOrNull()?.takeIf { it > 0 },
            carbsG = overrideCarbs.toIntOrNull()?.takeIf { it >= 0 },
            proteinG = overrideProtein.toIntOrNull()?.takeIf { it >= 0 },
            fatG = overrideFat.toIntOrNull()?.takeIf { it >= 0 },
            sugarG = overrideSugar.toIntOrNull()?.takeIf { it >= 0 },
        )
        scope.launch(Dispatchers.IO) {
            runCatching {
                NutritionGoalsStore.saveProfile(ctx, toPersist)
                NutritionGoalsStore.saveOverrides(ctx, overrides)
            }
            withContext(Dispatchers.Main) {
                submitting = false
                onToast("Saved")
                onBack()
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Nutrition goals", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // ===== 1. YOUR PROFILE =======================================
            SectionHeader("Your profile")

            // Sex — two pills.
            LabelText(text = "Sex")
            ChipRow {
                PillChip(
                    label = "Male",
                    selected = sex == Sex.Male,
                    onClick = { sex = if (sex == Sex.Male) null else Sex.Male },
                )
                PillChip(
                    label = "Female",
                    selected = sex == Sex.Female,
                    onClick = { sex = if (sex == Sex.Female) null else Sex.Female },
                )
            }

            OhdField(
                label = "Age (years)",
                value = age,
                onValueChange = { age = it.filter { ch -> ch.isDigit() }.take(3) },
                placeholder = "e.g. 32",
                keyboardType = KeyboardType.Number,
            )
            OhdField(
                label = "Height (cm)",
                value = heightCm,
                onValueChange = { heightCm = sanitizeDecimalInput(it) },
                placeholder = "e.g. 175",
                keyboardType = KeyboardType.Decimal,
            )
            OhdField(
                label = "Weight (kg)",
                value = weightKg,
                onValueChange = { weightKg = sanitizeDecimalInput(it) },
                placeholder = "e.g. 72.5",
                keyboardType = KeyboardType.Decimal,
            )

            LabelText(text = "Activity")
            ChipRow {
                PillChip("Sedentary", activity == Activity.Sedentary) { activity = Activity.Sedentary }
                PillChip("Light", activity == Activity.Light) { activity = Activity.Light }
                PillChip("Moderate", activity == Activity.Moderate) { activity = Activity.Moderate }
                PillChip("Active", activity == Activity.Active) { activity = Activity.Active }
                PillChip("Very active", activity == Activity.VeryActive) { activity = Activity.VeryActive }
            }

            LabelText(text = "Goal")
            ChipRow {
                PillChip("Cut", goal == Goal.Cut) { goal = Goal.Cut }
                PillChip("Maintain", goal == Goal.Maintain) { goal = Goal.Maintain }
                PillChip("Bulk", goal == Goal.Bulk) { goal = Goal.Bulk }
            }

            // ===== 2. RECOMMENDED TARGETS ================================
            SectionHeader("Recommended targets")
            if (preview == null) {
                Text(
                    text = "Fill in sex / age / height / weight for personalised " +
                        "targets — we'll use WHO defaults until then.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
            } else {
                PreviewBlock(preview)
            }

            // ===== 3. OVERRIDES ==========================================
            SectionHeader("Overrides")
            Text(
                text = "Pin a number per macro. Leave blank to use the recommendation.",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
            OhdField(
                label = "Calories (kcal)",
                value = overrideKcal,
                onValueChange = { overrideKcal = it.filter { ch -> ch.isDigit() }.take(5) },
                placeholder = "= recommended",
                keyboardType = KeyboardType.Number,
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OhdField(
                    label = "Carbs (g)",
                    value = overrideCarbs,
                    onValueChange = { overrideCarbs = it.filter { ch -> ch.isDigit() }.take(4) },
                    placeholder = "= recommended",
                    keyboardType = KeyboardType.Number,
                    modifier = Modifier.weight(1f),
                )
                OhdField(
                    label = "Protein (g)",
                    value = overrideProtein,
                    onValueChange = { overrideProtein = it.filter { ch -> ch.isDigit() }.take(4) },
                    placeholder = "= recommended",
                    keyboardType = KeyboardType.Number,
                    modifier = Modifier.weight(1f),
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OhdField(
                    label = "Fat (g)",
                    value = overrideFat,
                    onValueChange = { overrideFat = it.filter { ch -> ch.isDigit() }.take(4) },
                    placeholder = "= recommended",
                    keyboardType = KeyboardType.Number,
                    modifier = Modifier.weight(1f),
                )
                OhdField(
                    label = "Sugar (g)",
                    value = overrideSugar,
                    onValueChange = { overrideSugar = it.filter { ch -> ch.isDigit() }.take(4) },
                    placeholder = "= recommended",
                    keyboardType = KeyboardType.Number,
                    modifier = Modifier.weight(1f),
                )
            }

            // ===== Save =================================================
            OhdButton(
                label = if (submitting) "Saving…" else "Save",
                onClick = onSave,
                enabled = !submitting,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------

/**
 * Inline section header — mirrors [com.ohd.connect.ui.screens.FoodCreateScreen]
 * `OhdSectionHeaderInline`: same Inter 11 / 500 / Muted / letterSpacing 2,
 * but with only vertical padding because the parent column already supplies
 * the 16 dp horizontal gutter.
 */
@Composable
private fun SectionHeader(text: String) {
    Text(
        text = text.uppercase(),
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 11.sp,
        letterSpacing = 2.sp,
        color = OhdColors.Muted,
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
    )
}

@Composable
private fun LabelText(text: String) {
    Text(
        text = text,
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 13.sp,
        color = OhdColors.Ink,
    )
}

/**
 * Row container for a pill cluster. `space-between`-ish via `spacedBy(8.dp)`
 * so a long row (five activity pills) survives awkwardness-free by virtue
 * of being short labels rendered in a horizontal scroll.
 */
@Composable
private fun ChipRow(content: @Composable () -> Unit) {
    val state = rememberScrollState()
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .horizontalScroll(state),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        content()
    }
}

/**
 * Pill / chip — visually matches [com.ohd.connect.ui.screens.RecentEventsScreen]
 * `FilterChip` (selected = `Ink` fill, white text; unselected = `BgElevated`
 * fill, muted text).
 */
@Composable
private fun PillChip(
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
 * Compact 5-row preview of the recommended kcal + 4 macros. We render as
 * a flat list rather than reusing [com.ohd.connect.ui.components.OhdNutriGauge]
 * because the Settings screen wants the numbers themselves at the foreground,
 * not the donut/progress chrome.
 */
@Composable
private fun PreviewBlock(t: NutritionTargets) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, RoundedCornerShape(8.dp))
            .padding(horizontal = 12.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        PreviewRow("Calories", "${t.kcal} kcal")
        PreviewRow("Carbs", "${t.carbsG} g")
        PreviewRow("Protein", "${t.proteinG} g")
        PreviewRow("Fat", "${t.fatG} g")
        PreviewRow("Sugar", "${t.sugarG} g")
    }
}

@Composable
private fun PreviewRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
        Text(
            text = value,
            fontFamily = OhdMono,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
    }
}

/** Format a non-negative double with at most one decimal, dropping `.0`. */
private fun formatDecimal(value: Double): String {
    val rounded = (value * 10.0).toLong() / 10.0
    return if (rounded == rounded.toLong().toDouble()) {
        rounded.toLong().toString()
    } else {
        // Avoid Locale-dependent comma; we always parse with `.toDoubleOrNull()`
        // which only accepts `.`.
        "%.1f".format(java.util.Locale.US, rounded)
    }
}

/** Allow digits + at most one `.`. Caps length so the field stays sensible. */
private fun sanitizeDecimalInput(raw: String): String {
    var sawDot = false
    val sb = StringBuilder()
    for (ch in raw) {
        when {
            ch.isDigit() -> sb.append(ch)
            ch == '.' && !sawDot -> {
                sawDot = true
                sb.append('.')
            }
            ch == ',' && !sawDot -> {
                // Friendly: treat a typed comma as a decimal point so users
                // on locales that show "1,5" don't get silently rejected.
                sawDot = true
                sb.append('.')
            }
        }
        if (sb.length >= 6) break
    }
    return sb.toString()
}
