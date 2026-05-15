package com.ohd.connect.ui.screens

import com.ohd.connect.data.OpenFoodFacts

/**
 * Per-100 g nutrition facts for a single food item.
 *
 * The first five fields (kcal + four macros) are the ones every dictionary
 * entry fills in. The remaining vitamins / minerals / sub-fats are
 * opportunistically populated from the OpenFoodFacts response — they
 * default to `0.0` and are only surfaced in the "Show more" panel when at
 * least one row in a group has a non-zero value.
 *
 * Units:
 *  - `kcal` — kilocalories
 *  - `*G` fields — grams
 *  - `*Mg` fields — milligrams (OFF gives most micronutrients in grams, we
 *    convert at parse time)
 *  - `vitaminDMcg` — micrograms (OFF stores in grams; converted at parse time)
 */
data class NutritionFacts(
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
    /**
     * Every `_100g` nutriment OFF returned, keyed by its OFF name (e.g.
     * `caffeine`, `vitamin-b12`, `omega-3-fat`, `phosphorus`) with values in
     * the unit OFF uses (grams for macros/minerals, IU/µg for some vitamins).
     * The typed fields above are convenience accessors for the headline rows
     * the UI gauges show; this map is the full payload everything else
     * extends from (food.eaten event, "Show all" panel, LLM tool replies).
     *
     * Empty for in-app dictionary entries and for products OFF doesn't have
     * nutrition data for.
     */
    val nutrimentsPer100g: Map<String, Double> = emptyMap(),
) {
    /**
     * Flat key → numeric-value map used by [FoodNutritionPanel] to render
     * extended-nutrient rows generically. Keys mirror the field names so
     * downstream code can iterate without recompiling against this class.
     *
     * Calories are reported as a `Double` for shape uniformity; UI code
     * formats them as integer kcal.
     */
    val total: Map<String, Double>
        get() = linkedMapOf(
            "kcal" to kcal.toDouble(),
            "carbsG" to carbsG,
            "proteinG" to proteinG,
            "fatG" to fatG,
            "sugarG" to sugarG,
            "fiberG" to fiberG,
            "saturatedFatG" to saturatedFatG,
            "transFatG" to transFatG,
            "sodiumMg" to sodiumMg,
            "cholesterolMg" to cholesterolMg,
            "potassiumMg" to potassiumMg,
            "calciumMg" to calciumMg,
            "ironMg" to ironMg,
            "vitaminCMg" to vitaminCMg,
            "vitaminDMcg" to vitaminDMcg,
            "caffeineMg" to caffeineMg,
        )
}

/** Named pre-portion ("Box (50g)", "Bowl (180g)") for the detail screen chips. */
data class Serving(val name: String, val grams: Double)

/**
 * One entry in the in-app stub food dictionary used by [FoodSearchScreen]
 * and [FoodDetailScreen].
 *
 * v1 doesn't ship the OpenFoodFacts integration described in spec §4.7; we
 * filter this small fixed list by name instead. Each row carries:
 *
 *  - [description] — one-sentence OFF-flavored blurb shown on the detail
 *    screen.
 *  - [per100g] — macros normalised to 100 g, the basis for the rule-of-three
 *    computation in [FoodDetailScreen].
 *  - [packageServing] / [defaultPortion] — optional preset chips on the
 *    detail screen. A user can always fall back to the "Custom (g)" chip and
 *    type any amount.
 */
data class FoodItem(
    val name: String,
    val brand: String? = null,
    val source: String = "in-app dictionary",
    val description: String,
    val per100g: NutritionFacts,
    val packageServing: Serving? = null,
    val defaultPortion: Serving? = null,
    /**
     * Composition data harvested from OpenFoodFacts — empty for the in-app
     * dictionary, populated for live scans. All tags are stripped of OFF's
     * language prefix (`en:e330` → `e330`).
     *
     *  - [additives] — E-numbers (`e330`, `e150`, …).
     *  - [allergens] — declared allergens (`milk`, `gluten`).
     *  - [traces] — may-contain disclosures.
     *  - [ingredients] — full ingredient hierarchy, capped at 50 entries.
     *  - [ingredientsAnalysis] — `vegan`, `palm-oil-free`, …
     *  - [labels] — bio / fairtrade / "not advised for children" / …
     *  - [novaGroup] — 1 (unprocessed) to 4 (ultra-processed).
     *  - [nutriScore] — Nutri-Score letter `a`..`e` (lowercase).
     *  - [ecoScore] — Eco-Score letter `a`..`e` or `not-applicable`.
     */
    val additives: List<String> = emptyList(),
    val allergens: List<String> = emptyList(),
    val traces: List<String> = emptyList(),
    val ingredients: List<String> = emptyList(),
    val ingredientsAnalysis: List<String> = emptyList(),
    val labels: List<String> = emptyList(),
    val novaGroup: Int? = null,
    val nutriScore: String? = null,
    val ecoScore: String? = null,
)

/**
 * ~21 sample entries spanning the typical day (breakfast, lunch, snacks,
 * dinner). The list is small and the filter is naive — `name.contains(query,
 * ignoreCase = true)` — so the in-memory cost is irrelevant.
 *
 * Macros are reasonable approximations from publicly-available nutrition
 * tables. They're stub data — accuracy good enough for a demo, not a clinical
 * food log. Replace with live OpenFoodFacts calls once the OFF agent ships.
 */
val FoodDictionary: List<FoodItem> = listOf(
    FoodItem(
        name = "Oat porridge with banana",
        description = "Rolled oats cooked with milk or water, topped with sliced banana — a classic slow-release-carb breakfast.",
        per100g = NutritionFacts(kcal = 105, carbsG = 18.0, proteinG = 3.5, fatG = 2.0, sugarG = 5.0),
        defaultPortion = Serving("Bowl", 360.0),
    ),
    FoodItem(
        name = "Oat porridge — Quaker",
        brand = "Quaker",
        description = "Quick-cook rolled oats from Quaker. Whole-grain, no added sugar in the dry product.",
        per100g = NutritionFacts(kcal = 379, carbsG = 67.0, proteinG = 13.5, fatG = 6.5, sugarG = 1.0),
        packageServing = Serving("Sachet (40g)", 40.0),
        defaultPortion = Serving("Bowl (50g dry)", 50.0),
    ),
    FoodItem(
        name = "Greek yoghurt 200 g",
        description = "Strained yoghurt — higher protein than regular, mild sour taste.",
        per100g = NutritionFacts(kcal = 60, carbsG = 4.0, proteinG = 10.0, fatG = 0.5, sugarG = 4.0),
        packageServing = Serving("Cup (200g)", 200.0),
        defaultPortion = Serving("Bowl", 150.0),
    ),
    FoodItem(
        name = "Banana, medium",
        description = "Medium ripe banana, ~120 g without skin. Quick-release carbs and a hit of potassium.",
        per100g = NutritionFacts(kcal = 89, carbsG = 23.0, proteinG = 1.1, fatG = 0.3, sugarG = 12.0),
        defaultPortion = Serving("1 medium", 120.0),
    ),
    FoodItem(
        name = "Apple, medium",
        description = "Medium apple, ~180 g with skin on. Fibre-forward whole fruit.",
        per100g = NutritionFacts(kcal = 52, carbsG = 14.0, proteinG = 0.3, fatG = 0.2, sugarG = 10.0),
        defaultPortion = Serving("1 medium", 180.0),
    ),
    FoodItem(
        name = "Chicken breast 150 g",
        description = "Skinless boneless chicken breast, grilled or pan-seared. Lean protein staple.",
        per100g = NutritionFacts(kcal = 165, carbsG = 0.0, proteinG = 31.0, fatG = 3.6, sugarG = 0.0),
        defaultPortion = Serving("1 fillet", 150.0),
    ),
    FoodItem(
        name = "Chicken thigh 150 g",
        description = "Skinless boneless chicken thigh — fattier and more flavourful than breast.",
        per100g = NutritionFacts(kcal = 209, carbsG = 0.0, proteinG = 26.0, fatG = 11.0, sugarG = 0.0),
        defaultPortion = Serving("1 thigh", 150.0),
    ),
    FoodItem(
        name = "Salmon fillet 150 g",
        description = "Atlantic salmon fillet, baked or pan-seared. Omega-3-rich oily fish.",
        per100g = NutritionFacts(kcal = 187, carbsG = 0.0, proteinG = 20.0, fatG = 12.0, sugarG = 0.0),
        defaultPortion = Serving("1 fillet", 150.0),
    ),
    FoodItem(
        name = "Boiled egg, large",
        description = "Hard-boiled large hen's egg, ~50 g. Complete protein with most of the fat in the yolk.",
        per100g = NutritionFacts(kcal = 155, carbsG = 1.1, proteinG = 13.0, fatG = 11.0, sugarG = 1.1),
        defaultPortion = Serving("1 large", 50.0),
    ),
    FoodItem(
        name = "Avocado, half",
        description = "Half a Hass avocado, ~100 g. Mostly monounsaturated fat plus fibre.",
        per100g = NutritionFacts(kcal = 160, carbsG = 9.0, proteinG = 2.0, fatG = 15.0, sugarG = 0.7),
        defaultPortion = Serving("Half", 100.0),
    ),
    FoodItem(
        name = "Brown rice, cooked 200 g",
        description = "Long-grain brown rice, boiled. Whole-grain side carb.",
        per100g = NutritionFacts(kcal = 124, carbsG = 26.0, proteinG = 2.7, fatG = 1.0, sugarG = 0.4),
        defaultPortion = Serving("Bowl", 200.0),
    ),
    FoodItem(
        name = "Quinoa, cooked 200 g",
        description = "Cooked quinoa — pseudo-cereal with all nine essential amino acids.",
        per100g = NutritionFacts(kcal = 120, carbsG = 21.0, proteinG = 4.4, fatG = 1.9, sugarG = 0.9),
        defaultPortion = Serving("Bowl", 200.0),
    ),
    FoodItem(
        name = "Sweet potato, baked 200 g",
        description = "Baked sweet potato with skin. Beta-carotene-rich complex carb.",
        per100g = NutritionFacts(kcal = 90, carbsG = 21.0, proteinG = 2.0, fatG = 0.2, sugarG = 6.5),
        defaultPortion = Serving("1 medium", 200.0),
    ),
    FoodItem(
        name = "Spinach salad 100 g",
        description = "Fresh baby spinach leaves, no dressing. Volume eating with iron and folate.",
        per100g = NutritionFacts(kcal = 23, carbsG = 3.6, proteinG = 2.9, fatG = 0.4, sugarG = 0.4),
        defaultPortion = Serving("Bowl", 100.0),
    ),
    FoodItem(
        name = "Almonds, 30 g handful",
        description = "Whole raw almonds. Snack-friendly source of vitamin E and monounsaturated fat.",
        per100g = NutritionFacts(kcal = 579, carbsG = 22.0, proteinG = 21.0, fatG = 50.0, sugarG = 4.4),
        packageServing = Serving("Handful (30g)", 30.0),
        defaultPortion = Serving("Handful", 30.0),
    ),
    FoodItem(
        name = "Olive oil, 1 tbsp",
        description = "Extra-virgin olive oil. Nearly all monounsaturated fat — used as cooking medium or dressing.",
        per100g = NutritionFacts(kcal = 884, carbsG = 0.0, proteinG = 0.0, fatG = 100.0, sugarG = 0.0),
        defaultPortion = Serving("1 tbsp", 14.0),
    ),
    FoodItem(
        name = "Whole-grain toast slice",
        description = "Standard slice (~35 g) of whole-grain bread, toasted.",
        per100g = NutritionFacts(kcal = 247, carbsG = 41.0, proteinG = 13.0, fatG = 4.2, sugarG = 5.0),
        defaultPortion = Serving("1 slice", 35.0),
    ),
    FoodItem(
        name = "Peanut butter, 1 tbsp",
        description = "Natural peanut butter, no added sugar or palm oil. Calorie-dense plant protein.",
        per100g = NutritionFacts(kcal = 588, carbsG = 20.0, proteinG = 25.0, fatG = 50.0, sugarG = 9.0),
        defaultPortion = Serving("1 tbsp", 16.0),
    ),
    FoodItem(
        name = "Espresso, single",
        description = "Single shot of espresso, ~30 ml. No added milk or sugar.",
        per100g = NutritionFacts(kcal = 9, carbsG = 1.7, proteinG = 0.1, fatG = 0.2, sugarG = 0.0),
        defaultPortion = Serving("1 shot (30ml)", 30.0),
    ),
    FoodItem(
        name = "Latte, 250 ml",
        description = "Espresso topped with steamed whole milk. ~250 ml café-style cup.",
        per100g = NutritionFacts(kcal = 48, carbsG = 4.6, proteinG = 2.6, fatG = 2.0, sugarG = 4.6),
        packageServing = Serving("Cup (250ml)", 250.0),
        defaultPortion = Serving("Cup", 250.0),
    ),
    FoodItem(
        name = "Dark chocolate 20 g",
        description = "70% cocoa dark chocolate square (~20 g). Rich in flavanols and saturated fat.",
        per100g = NutritionFacts(kcal = 550, carbsG = 46.0, proteinG = 7.5, fatG = 35.0, sugarG = 35.0),
        packageServing = Serving("Square (20g)", 20.0),
        defaultPortion = Serving("Square", 20.0),
    ),
)

/** Case-insensitive name filter used by [FoodSearchScreen]. */
fun searchFoodDictionary(query: String): List<FoodItem> {
    val trimmed = query.trim()
    if (trimmed.isEmpty()) return FoodDictionary
    return FoodDictionary.filter { it.name.contains(trimmed, ignoreCase = true) }
}

/**
 * Look up a single dictionary entry by exact name (case-sensitive). Falls
 * back to [OpenFoodFacts.cache] so items resolved over the network during
 * search are still findable from [FoodDetailScreen] after navigation.
 */
fun foodByName(name: String): FoodItem? =
    FoodDictionary.firstOrNull { it.name == name }
        ?: OpenFoodFacts.cache.values.firstOrNull { it.name == name }
