package com.ohd.connect.data

import android.content.Context
import com.ohd.connect.ui.screens.FoodItem
import com.ohd.connect.ui.screens.NutritionFacts
import com.ohd.connect.ui.screens.Packaging
import com.ohd.connect.ui.screens.Serving
import org.json.JSONArray
import org.json.JSONObject
import java.security.SecureRandom

/**
 * On-device store for foods the user hand-creates from
 * [com.ohd.connect.ui.screens.FoodCreateScreen], for cases where neither the
 * in-app dictionary nor OpenFoodFacts has what they ate.
 *
 * ## Persistence shape (v2)
 *
 * A single JSON-array string under [KEY_CUSTOM_FOODS_V2] inside the same
 * Keystore-wrapped `EncryptedSharedPreferences` file used by [Auth] (we go
 * through [Auth.securePrefs] so we inherit the AES-256-GCM master key
 * without a second Keystore round-trip).
 *
 * Each row mirrors every field on [FoodItem], [NutritionFacts] and
 * [Serving] so a custom row round-trips losslessly:
 *
 *     {
 *       "id":          "custom:01HXR…",
 *       "name":        "Homemade granola",
 *       "brand":       "Self-made",                // optional
 *       "source":      "user-created",
 *       "description": "Oats / nuts / honey …",
 *       "per100g": {
 *           "kcal":          475,
 *           "carbsG":        55.0,
 *           "proteinG":      12.0,
 *           "fatG":          22.0,
 *           "sugarG":        18.0,
 *           "fiberG":        7.0,
 *           "saturatedFatG": 3.0,
 *           "transFatG":     0.0,
 *           "sodiumMg":      120.0,
 *           "cholesterolMg": 0.0,
 *           "potassiumMg":   320.0,
 *           "calciumMg":     80.0,
 *           "ironMg":        2.1,
 *           "vitaminCMg":    0.0,
 *           "vitaminDMcg":   0.0,
 *           "caffeineMg":    0.0,
 *           "nutrimentsPer100g": { "phosphorus": 0.14, … }   // optional
 *       },
 *       "servings": [                                        // optional
 *           { "name": "Bag", "grams": 400.0 },
 *           { "name": "Bowl", "grams": 60.0 }
 *       ],
 *       "additives":            ["e330"],                    // optional
 *       "allergens":            ["gluten", "milk"],          // optional
 *       "traces":               ["nuts"],                    // optional
 *       "ingredients":          ["oats", "honey", …],        // optional
 *       "ingredientsAnalysis":  ["vegetarian"],              // optional
 *       "labels":               ["organic"],                 // optional
 *       "novaGroup":            1,                           // optional
 *       "nutriScore":           "a",                         // optional
 *       "ecoScore":             "b"                          // optional
 *     }
 *
 * `id` is purely a storage handle — [FoodItem] itself does not carry an id
 * field (the user explicitly asked not to change the data class). Callers
 * that want to delete a row pass the id back to [remove].
 *
 * ## v1 → v2 migration
 *
 * Earlier betas wrote the same JSON shape under [KEY_CUSTOM_FOODS_V1] with
 * only the basics + five-macro nutrition. On the first read where v2 is
 * absent but v1 is present we parse v1 (defaulted to zero for the
 * extended fields), write the rows back as v2, and delete v1. The
 * migration is best-effort and silent — on parse failure v1 stays around
 * so a follow-up beta can retry.
 *
 * Beta-only stack: the user wipes data each install for most flows, but
 * the cheap migration costs nothing and keeps anyone who upgraded
 * in-place from losing their hand-created entries.
 */
object CustomFoodStore {

    private const val KEY_CUSTOM_FOODS_V1 = "custom_foods_v1"
    private const val KEY_CUSTOM_FOODS_V2 = "custom_foods_v2"
    private const val KEY_CUSTOM_FOODS_V3 = "custom_foods_v3"
    private const val KEY_CUSTOM_FOODS_V4 = "custom_foods_v4"
    private const val ID_PREFIX = "custom:"

    /** All foods the user has created, newest-first. */
    fun all(ctx: Context): List<FoodItem> = readRows(ctx).map { it.food }

    /** Substring (case-insensitive) match on name + brand. */
    fun search(ctx: Context, query: String): List<FoodItem> {
        val q = query.trim()
        val rows = all(ctx)
        if (q.isEmpty()) return rows
        return rows.filter { food ->
            food.name.contains(q, ignoreCase = true) ||
                (food.brand?.contains(q, ignoreCase = true) == true)
        }
    }

    /**
     * Persist a new food. Returns the food (the id assignment is internal —
     * [FoodItem] has no id field, so the public return value is identical
     * to the input).
     */
    fun add(ctx: Context, food: FoodItem): FoodItem {
        val current = readRows(ctx).toMutableList()
        val newRow = Row(id = mintId(), food = food)
        // Newest-first.
        current.add(0, newRow)
        writeRows(ctx, current)
        return food
    }

    /** Remove by stable id. Returns true if a row was removed. */
    fun remove(ctx: Context, id: String): Boolean {
        val current = readRows(ctx)
        val filtered = current.filterNot { it.id == id }
        if (filtered.size == current.size) return false
        writeRows(ctx, filtered)
        return true
    }

    // ---- internals --------------------------------------------------------

    /** One persisted row — id + the FoodItem it wraps. */
    private data class Row(val id: String, val food: FoodItem)

    private fun readRows(ctx: Context): List<Row> {
        val prefs = Auth.securePrefs(ctx)
        // v4 is the current shape. Earlier versions are forward-compatible
        // — every field added since is optional with a sensible default,
        // and the v3→v4 shape change (packageServing + defaultPortion → a
        // single `servings` list) is reconciled inside [rowFromJson] which
        // accepts either form. So reading v3 / v2 / v1 through the same
        // parser fills the new fields with their data-class defaults,
        // merges the two legacy serving fields into a list, then writes
        // back as v4 and cleans up the old key. Migration is one-shot per
        // key.
        prefs.getString(KEY_CUSTOM_FOODS_V4, null)?.takeIf { it.isNotBlank() }?.let {
            return parseArray(it)
        }
        for (legacyKey in listOf(KEY_CUSTOM_FOODS_V3, KEY_CUSTOM_FOODS_V2, KEY_CUSTOM_FOODS_V1)) {
            prefs.getString(legacyKey, null)?.takeIf { it.isNotBlank() }?.let { legacy ->
                val parsed = parseArray(legacy)
                if (parsed.isEmpty()) return emptyList()
                writeRows(ctx, parsed)
                prefs.edit().remove(legacyKey).apply()
                return parsed
            }
        }
        return emptyList()
    }

    private fun parseArray(raw: String): List<Row> = runCatching {
        val arr = JSONArray(raw)
        (0 until arr.length()).mapNotNull { i -> rowFromJson(arr.getJSONObject(i)) }
    }.getOrDefault(emptyList())

    private fun writeRows(ctx: Context, rows: List<Row>) {
        val arr = JSONArray()
        rows.forEach { arr.put(rowToJson(it)) }
        Auth.securePrefs(ctx).edit()
            .putString(KEY_CUSTOM_FOODS_V4, arr.toString())
            .apply()
    }

    // ---- (de)serialisation ----------------------------------------------

    private fun rowToJson(row: Row): JSONObject {
        val f = row.food
        val obj = JSONObject()
            .put("id", row.id)
            .put("name", f.name)
            .put("source", f.source)
            .put("description", f.description)
            .put("per100g", nutritionToJson(f.per100g))
        if (!f.brand.isNullOrBlank()) obj.put("brand", f.brand)
        if (!f.barcode.isNullOrBlank()) obj.put("barcode", f.barcode)
        if (f.servings.isNotEmpty()) {
            obj.put("servings", JSONArray().apply {
                f.servings.forEach { put(servingToJson(it)) }
            })
        }
        f.packaging?.takeUnless { it.isBlank }?.let { obj.put("packaging", packagingToJson(it)) }
        if (f.additives.isNotEmpty()) obj.put("additives", JSONArray(f.additives))
        if (f.allergens.isNotEmpty()) obj.put("allergens", JSONArray(f.allergens))
        if (f.traces.isNotEmpty()) obj.put("traces", JSONArray(f.traces))
        if (f.ingredients.isNotEmpty()) obj.put("ingredients", JSONArray(f.ingredients))
        if (f.ingredientsAnalysis.isNotEmpty()) {
            obj.put("ingredientsAnalysis", JSONArray(f.ingredientsAnalysis))
        }
        if (f.labels.isNotEmpty()) obj.put("labels", JSONArray(f.labels))
        f.novaGroup?.let { obj.put("novaGroup", it) }
        f.nutriScore?.let { obj.put("nutriScore", it) }
        f.ecoScore?.let { obj.put("ecoScore", it) }
        return obj
    }

    private fun rowFromJson(obj: JSONObject): Row? = runCatching {
        val id = obj.optString("id").takeIf { it.isNotEmpty() } ?: mintId()
        val name = obj.optString("name").takeIf { it.isNotEmpty() } ?: return@runCatching null
        val brand = obj.optString("brand").takeIf { it.isNotEmpty() }
        val barcode = obj.optString("barcode").takeIf { it.isNotEmpty() }
        val source = obj.optString("source").takeIf { it.isNotEmpty() } ?: "user-created"
        val description = obj.optString("description")
        val per100g = nutritionFromJson(obj.optJSONObject("per100g"))
        // v4 shape: `servings: [{name, grams}, …]`. v3 and earlier wrote
        // packageServing + defaultPortion as separate optional objects;
        // we merge whichever are present into the new list, packageServing
        // first to preserve original ordering.
        val servings: List<Serving> = buildList {
            obj.optJSONArray("servings")?.let { arr ->
                for (i in 0 until arr.length()) {
                    arr.optJSONObject(i)?.let { servingFromJson(it) }?.let(::add)
                }
            }
            if (isEmpty()) {
                obj.optJSONObject("packageServing")?.let { servingFromJson(it) }?.let(::add)
                obj.optJSONObject("defaultPortion")?.let { servingFromJson(it) }?.let(::add)
            }
        }
        val packaging = obj.optJSONObject("packaging")?.let { packagingFromJson(it) }
        Row(
            id = id,
            food = FoodItem(
                name = name,
                brand = brand,
                barcode = barcode,
                source = source,
                description = description,
                per100g = per100g,
                servings = servings,
                packaging = packaging,
                additives = stringListFrom(obj, "additives"),
                allergens = stringListFrom(obj, "allergens"),
                traces = stringListFrom(obj, "traces"),
                ingredients = stringListFrom(obj, "ingredients"),
                ingredientsAnalysis = stringListFrom(obj, "ingredientsAnalysis"),
                labels = stringListFrom(obj, "labels"),
                novaGroup = if (obj.has("novaGroup") && !obj.isNull("novaGroup")) {
                    obj.optInt("novaGroup", 0).takeIf { it in 1..4 }
                } else {
                    null
                },
                nutriScore = obj.optString("nutriScore").takeIf { it.isNotEmpty() },
                ecoScore = obj.optString("ecoScore").takeIf { it.isNotEmpty() },
            ),
        )
    }.getOrNull()

    private fun nutritionToJson(n: NutritionFacts): JSONObject {
        val obj = JSONObject()
            .put("kcal", n.kcal)
            .put("carbsG", n.carbsG)
            .put("proteinG", n.proteinG)
            .put("fatG", n.fatG)
            .put("sugarG", n.sugarG)
            .put("fiberG", n.fiberG)
            .put("saturatedFatG", n.saturatedFatG)
            .put("transFatG", n.transFatG)
            .put("sodiumMg", n.sodiumMg)
            .put("cholesterolMg", n.cholesterolMg)
            .put("potassiumMg", n.potassiumMg)
            .put("calciumMg", n.calciumMg)
            .put("ironMg", n.ironMg)
            .put("vitaminCMg", n.vitaminCMg)
            .put("vitaminDMcg", n.vitaminDMcg)
            .put("caffeineMg", n.caffeineMg)
        if (n.nutrimentsPer100g.isNotEmpty()) {
            val nm = JSONObject()
            n.nutrimentsPer100g.forEach { (k, v) -> nm.put(k, v) }
            obj.put("nutrimentsPer100g", nm)
        }
        return obj
    }

    private fun nutritionFromJson(obj: JSONObject?): NutritionFacts {
        if (obj == null) {
            return NutritionFacts(kcal = 0, carbsG = 0.0, proteinG = 0.0, fatG = 0.0, sugarG = 0.0)
        }
        val nutriments: Map<String, Double> = obj.optJSONObject("nutrimentsPer100g")
            ?.let { nm ->
                val out = linkedMapOf<String, Double>()
                val keys = nm.keys()
                while (keys.hasNext()) {
                    val k = keys.next()
                    out[k] = nm.optDouble(k, 0.0)
                }
                out
            }
            ?: emptyMap()
        return NutritionFacts(
            kcal = obj.optInt("kcal", 0),
            carbsG = obj.optDouble("carbsG", 0.0),
            proteinG = obj.optDouble("proteinG", 0.0),
            fatG = obj.optDouble("fatG", 0.0),
            sugarG = obj.optDouble("sugarG", 0.0),
            fiberG = obj.optDouble("fiberG", 0.0),
            saturatedFatG = obj.optDouble("saturatedFatG", 0.0),
            transFatG = obj.optDouble("transFatG", 0.0),
            sodiumMg = obj.optDouble("sodiumMg", 0.0),
            cholesterolMg = obj.optDouble("cholesterolMg", 0.0),
            potassiumMg = obj.optDouble("potassiumMg", 0.0),
            calciumMg = obj.optDouble("calciumMg", 0.0),
            ironMg = obj.optDouble("ironMg", 0.0),
            vitaminCMg = obj.optDouble("vitaminCMg", 0.0),
            vitaminDMcg = obj.optDouble("vitaminDMcg", 0.0),
            caffeineMg = obj.optDouble("caffeineMg", 0.0),
            nutrimentsPer100g = nutriments,
        )
    }

    private fun servingToJson(s: Serving): JSONObject = JSONObject()
        .put("name", s.name)
        .put("grams", s.grams)

    private fun servingFromJson(obj: JSONObject): Serving? {
        val name = obj.optString("name").takeIf { it.isNotEmpty() } ?: return null
        val grams = obj.optDouble("grams", Double.NaN)
        if (grams.isNaN()) return null
        return Serving(name = name, grams = grams)
    }

    private fun packagingToJson(p: Packaging): JSONObject {
        val obj = JSONObject()
        p.material?.let { obj.put("material", it) }
        p.format?.let { obj.put("format", it) }
        p.recyclable?.let { obj.put("recyclable", it) }
        p.recycledContentPct?.let { obj.put("recycledContentPct", it) }
        if (!p.notes.isNullOrBlank()) obj.put("notes", p.notes)
        return obj
    }

    private fun packagingFromJson(obj: JSONObject): Packaging {
        val recyclable = if (obj.has("recyclable") && !obj.isNull("recyclable")) {
            obj.optBoolean("recyclable")
        } else {
            null
        }
        val pct = if (obj.has("recycledContentPct") && !obj.isNull("recycledContentPct")) {
            obj.optInt("recycledContentPct", -1).takeIf { it in 0..100 }
        } else {
            null
        }
        return Packaging(
            material = obj.optString("material").takeIf { it.isNotEmpty() },
            format = obj.optString("format").takeIf { it.isNotEmpty() },
            recyclable = recyclable,
            recycledContentPct = pct,
            notes = obj.optString("notes").takeIf { it.isNotEmpty() },
        )
    }

    private fun stringListFrom(obj: JSONObject, key: String): List<String> {
        val arr = obj.optJSONArray(key) ?: return emptyList()
        val out = ArrayList<String>(arr.length())
        for (i in 0 until arr.length()) {
            val s = arr.optString(i)
            if (s.isNotEmpty()) out += s
        }
        return out
    }

    /**
     * Mint a fresh `custom:<26-char>` id. Same Crockford-base32 shape as
     * [OhdAccountStore.newProfileUlid] — 10 char ms-timestamp + 16 random.
     * Unused field in [FoodItem] but kept as the persistence handle for
     * [remove].
     */
    private fun mintId(now: Long = System.currentTimeMillis()): String {
        val rng = SecureRandom()
        val alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
        val ts = StringBuilder()
        var n = now
        repeat(10) {
            ts.append(alphabet[(n and 31L).toInt()])
            n = n ushr 5
        }
        ts.reverse()
        val rand = StringBuilder()
        repeat(16) { rand.append(alphabet[rng.nextInt(alphabet.length)]) }
        return ID_PREFIX + ts.toString() + rand.toString()
    }
}
