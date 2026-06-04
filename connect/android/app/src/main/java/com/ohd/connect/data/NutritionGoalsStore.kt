package com.ohd.connect.data

import android.content.Context
import org.json.JSONObject
import kotlin.math.max
import kotlin.math.roundToInt

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/**
 * Editable user metabolic profile used to compute personalised nutrition
 * targets. All fields are nullable so an incomplete first-run profile still
 * round-trips cleanly — when any of [sex] / [ageYears] / [heightCm] /
 * [weightKg] is missing the math falls back to WHO defaults.
 *
 * [activity] and [goal] have non-null defaults because they're always
 * pickable from a pill row in the UI (a "best guess" Sedentary / Maintain
 * is friendlier than forcing a choice).
 */
data class NutritionProfile(
    val sex: Sex? = null,
    val ageYears: Int? = null,
    val heightCm: Double? = null,
    val weightKg: Double? = null,
    val activity: Activity = Activity.Sedentary,
    val goal: Goal = Goal.Maintain,
)

/** Biological sex — only used in the Mifflin–St Jeor BMR constant. */
enum class Sex { Male, Female }

/**
 * Physical-activity level. Multiplier values follow the FAO / Mifflin
 * conventions used in most consumer nutrition apps:
 *
 *   Sedentary  1.20  (desk job, little exercise)
 *   Light      1.375 (light exercise 1–3×/week)
 *   Moderate   1.55  (moderate 3–5×/week)
 *   Active     1.725 (heavy 6–7×/week)
 *   VeryActive 1.90  (physical job + training)
 */
enum class Activity { Sedentary, Light, Moderate, Active, VeryActive }

/** Energy-balance goal. Drives both the kcal scale and the protein g/kg. */
enum class Goal { Cut, Maintain, Bulk }

/**
 * Per-macro overrides. `null` means "no override — fall through to the
 * computed-from-profile value, or the WHO default if the profile is
 * incomplete". The UI presents each as an optional numeric field with
 * placeholder "= recommended".
 */
data class NutritionOverrides(
    val kcal: Int? = null,
    val carbsG: Int? = null,
    val proteinG: Int? = null,
    val fatG: Int? = null,
    val sugarG: Int? = null,
)

/**
 * Resolved daily targets the Food tab gauges render against. Always
 * five finite ints — never throws, never NaN.
 */
data class NutritionTargets(
    val kcal: Int,
    val carbsG: Int,
    val proteinG: Int,
    val fatG: Int,
    val sugarG: Int,
)

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/**
 * On-device store for the user's metabolic profile + per-macro overrides
 * + computed daily nutrition targets.
 *
 * Two JSON blobs in a plain `SharedPreferences` file:
 *
 *   ohd_nutrition_goals
 *     "profile"   → { "sex": "Male"|"Female"|null, "ageYears": Int|null,
 *                     "heightCm": Double|null, "weightKg": Double|null,
 *                     "activity": "Sedentary"|"Light"|"Moderate"|"Active"|"VeryActive",
 *                     "goal":     "Cut"|"Maintain"|"Bulk" }
 *     "overrides" → { "kcal": Int|null, "carbsG": Int|null,
 *                     "proteinG": Int|null, "fatG": Int|null,
 *                     "sugarG": Int|null }
 *
 * Plain (not encrypted) — this isn't sensitive data and mirrors
 * [ExcludedTypesStore] / [HealthConnectScheduler] which use the same
 * `Context.MODE_PRIVATE` file pattern.
 *
 * ## Resolution order for [effectiveTargets]
 *
 *   1. Per-macro override (`overrides.X` non-null)         → use it.
 *   2. Computed-from-profile (profile complete + finite)   → use it.
 *   3. WHO daily reference fallback                        → use it.
 *
 * Each macro is resolved independently so a user with only a kcal override
 * still picks up computed carbs/protein/fat/sugar from their profile.
 */
object NutritionGoalsStore {

    private const val PREFS_NAME = "ohd_nutrition_goals"
    private const val KEY_PROFILE = "profile"
    private const val KEY_OVERRIDES = "overrides"

    // WHO-style daily reference values used when the profile is incomplete
    // **and** no override is set. These match the public-facing daily-value
    // panels printed on packaging (Codex Alimentarius / WHO 2003).
    internal const val WHO_KCAL = 2000
    internal const val WHO_CARBS_G = 250
    internal const val WHO_PROTEIN_G = 100
    internal const val WHO_FAT_G = 67
    internal const val WHO_SUGAR_G = 50

    private fun prefs(ctx: Context) =
        ctx.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    // ---- Profile ---------------------------------------------------------

    fun loadProfile(ctx: Context): NutritionProfile {
        val raw = prefs(ctx).getString(KEY_PROFILE, null) ?: return NutritionProfile()
        return runCatching { profileFromJson(JSONObject(raw)) }.getOrDefault(NutritionProfile())
    }

    fun saveProfile(ctx: Context, profile: NutritionProfile) {
        prefs(ctx).edit()
            .putString(KEY_PROFILE, profileToJson(profile).toString())
            .apply()
    }

    // ---- Overrides -------------------------------------------------------

    fun loadOverrides(ctx: Context): NutritionOverrides {
        val raw = prefs(ctx).getString(KEY_OVERRIDES, null) ?: return NutritionOverrides()
        return runCatching { overridesFromJson(JSONObject(raw)) }
            .getOrDefault(NutritionOverrides())
    }

    fun saveOverrides(ctx: Context, overrides: NutritionOverrides) {
        prefs(ctx).edit()
            .putString(KEY_OVERRIDES, overridesToJson(overrides).toString())
            .apply()
    }

    // ---- Resolution ------------------------------------------------------

    /**
     * Effective per-macro targets for the Food tab gauges. Each macro is
     * resolved independently so the user can pin one number ("I want 200 g
     * protein") without abandoning the personalised computation for the
     * rest. Falls all the way through to WHO defaults if neither an
     * override nor a complete profile is available.
     */
    fun effectiveTargets(ctx: Context): NutritionTargets {
        val profile = loadProfile(ctx)
        val overrides = loadOverrides(ctx)
        val computed = computedTargets(profile)
        return NutritionTargets(
            kcal = overrides.kcal ?: computed?.kcal ?: WHO_KCAL,
            carbsG = overrides.carbsG ?: computed?.carbsG ?: WHO_CARBS_G,
            proteinG = overrides.proteinG ?: computed?.proteinG ?: WHO_PROTEIN_G,
            fatG = overrides.fatG ?: computed?.fatG ?: WHO_FAT_G,
            sugarG = overrides.sugarG ?: computed?.sugarG ?: WHO_SUGAR_G,
        )
    }

    /**
     * Personalised targets from the profile alone. Returns `null` when the
     * profile is incomplete (any of sex / age / height / weight missing),
     * so callers can render a "fill in the rest" hint.
     *
     * Sugar is always WHO 25 g (free-sugar guideline) regardless of goal —
     * tightening sugar is a public-health recommendation independent of
     * energy balance.
     */
    fun computedTargets(profile: NutritionProfile): NutritionTargets? {
        val sex = profile.sex ?: return null
        val age = profile.ageYears ?: return null
        val heightCm = profile.heightCm ?: return null
        val weightKg = profile.weightKg ?: return null
        if (age <= 0 || heightCm <= 0.0 || weightKg <= 0.0) return null
        if (!heightCm.isFinite() || !weightKg.isFinite()) return null

        // ---- Mifflin–St Jeor BMR ----
        val bmr = when (sex) {
            Sex.Male -> 10.0 * weightKg + 6.25 * heightCm - 5.0 * age + 5.0
            Sex.Female -> 10.0 * weightKg + 6.25 * heightCm - 5.0 * age - 161.0
        }
        val pal = palFor(profile.activity)
        val tdee = bmr * pal

        // ---- Goal scale on kcal ----
        val goalScale = when (profile.goal) {
            Goal.Cut -> 0.80
            Goal.Maintain -> 1.00
            Goal.Bulk -> 1.10
        }
        val kcal = (tdee * goalScale).roundToInt().coerceAtLeast(0)

        // ---- Macros (grams) ----
        // Protein g/kg bodyweight by goal — common evidence-backed splits.
        val proteinPerKg = when (profile.goal) {
            Goal.Cut -> 2.2
            Goal.Maintain -> 1.6
            Goal.Bulk -> 2.0
        }
        val proteinG = (weightKg * proteinPerKg).roundToInt().coerceAtLeast(0)
        val fatG = (weightKg * 1.0).roundToInt().coerceAtLeast(0)
        // Carbs fill the rest of the kcal budget: kcal - 4·P - 9·F, /4.
        val carbsKcal = kcal - proteinG * 4 - fatG * 9
        val carbsG = max(0, (carbsKcal / 4.0).roundToInt())
        val sugarG = 25  // WHO free-sugar guideline.

        return NutritionTargets(
            kcal = kcal,
            carbsG = carbsG,
            proteinG = proteinG,
            fatG = fatG,
            sugarG = sugarG,
        )
    }

    private fun palFor(activity: Activity): Double = when (activity) {
        Activity.Sedentary -> 1.2
        Activity.Light -> 1.375
        Activity.Moderate -> 1.55
        Activity.Active -> 1.725
        Activity.VeryActive -> 1.9
    }

    // ---- JSON (de)serialisation -----------------------------------------

    private fun profileToJson(p: NutritionProfile): JSONObject {
        val o = JSONObject()
        o.put("sex", p.sex?.name)
        o.put("ageYears", p.ageYears ?: JSONObject.NULL)
        o.put("heightCm", p.heightCm ?: JSONObject.NULL)
        o.put("weightKg", p.weightKg ?: JSONObject.NULL)
        o.put("activity", p.activity.name)
        o.put("goal", p.goal.name)
        return o
    }

    private fun profileFromJson(o: JSONObject): NutritionProfile {
        val sex = o.optString("sex").takeIf { it.isNotEmpty() && it != "null" }
            ?.let { runCatching { Sex.valueOf(it) }.getOrNull() }
        val age = if (o.has("ageYears") && !o.isNull("ageYears")) {
            o.optInt("ageYears", 0).takeIf { it > 0 }
        } else {
            null
        }
        val height = if (o.has("heightCm") && !o.isNull("heightCm")) {
            o.optDouble("heightCm", Double.NaN).takeIf { it.isFinite() && it > 0.0 }
        } else {
            null
        }
        val weight = if (o.has("weightKg") && !o.isNull("weightKg")) {
            o.optDouble("weightKg", Double.NaN).takeIf { it.isFinite() && it > 0.0 }
        } else {
            null
        }
        val activity = o.optString("activity").takeIf { it.isNotEmpty() }
            ?.let { runCatching { Activity.valueOf(it) }.getOrNull() }
            ?: Activity.Sedentary
        val goal = o.optString("goal").takeIf { it.isNotEmpty() }
            ?.let { runCatching { Goal.valueOf(it) }.getOrNull() }
            ?: Goal.Maintain
        return NutritionProfile(
            sex = sex,
            ageYears = age,
            heightCm = height,
            weightKg = weight,
            activity = activity,
            goal = goal,
        )
    }

    private fun overridesToJson(o: NutritionOverrides): JSONObject {
        val obj = JSONObject()
        obj.put("kcal", o.kcal ?: JSONObject.NULL)
        obj.put("carbsG", o.carbsG ?: JSONObject.NULL)
        obj.put("proteinG", o.proteinG ?: JSONObject.NULL)
        obj.put("fatG", o.fatG ?: JSONObject.NULL)
        obj.put("sugarG", o.sugarG ?: JSONObject.NULL)
        return obj
    }

    private fun overridesFromJson(obj: JSONObject): NutritionOverrides =
        NutritionOverrides(
            kcal = obj.optIntOrNull("kcal"),
            carbsG = obj.optIntOrNull("carbsG"),
            proteinG = obj.optIntOrNull("proteinG"),
            fatG = obj.optIntOrNull("fatG"),
            sugarG = obj.optIntOrNull("sugarG"),
        )

    private fun JSONObject.optIntOrNull(key: String): Int? {
        if (!has(key) || isNull(key)) return null
        val v = optInt(key, Int.MIN_VALUE)
        return if (v == Int.MIN_VALUE) null else v
    }
}
