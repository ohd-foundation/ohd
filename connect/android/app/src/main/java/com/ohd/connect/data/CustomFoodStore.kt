package com.ohd.connect.data

import android.content.Context
import com.ohd.connect.ui.screens.FoodItem
import com.ohd.connect.ui.screens.NutritionFacts
import org.json.JSONArray
import org.json.JSONObject
import java.security.SecureRandom

/**
 * On-device store for foods the user hand-creates from
 * [com.ohd.connect.ui.screens.FoodCreateScreen], for cases where neither the
 * in-app dictionary nor OpenFoodFacts has what they ate.
 *
 * Persistence shape — a single JSON-array string under
 * [KEY_CUSTOM_FOODS] inside the same Keystore-wrapped
 * `EncryptedSharedPreferences` file used by [Auth] (we go through
 * [Auth.securePrefs] so we inherit the AES-256-GCM master key without a
 * second Keystore round-trip).
 *
 * Each row is:
 *
 *     {
 *       "id":          "custom:01HXR…",
 *       "name":        "Homemade granola",
 *       "brand":       "—",                 // optional
 *       "source":      "user-created",
 *       "description": "Oat / nut / honey …",
 *       "per100g": {
 *           "kcal":     475,
 *           "carbsG":   55.0,
 *           "proteinG": 12.0,
 *           "fatG":     22.0,
 *           "sugarG":   18.0
 *       }
 *     }
 *
 * `id` is purely a storage handle — [FoodItem] itself does not carry an id
 * field (the user explicitly asked not to change the data class). Callers
 * that want to delete a row pass the id back to [remove].
 *
 * Beta-only stack: the user wipes data each install (see project memory), so
 * no schema migration concerns — we can rev the JSON shape freely; just bump
 * the key suffix.
 */
object CustomFoodStore {

    private const val KEY_CUSTOM_FOODS = "custom_foods_v1"
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
        val raw = Auth.securePrefs(ctx).getString(KEY_CUSTOM_FOODS, null)
        if (raw.isNullOrBlank()) return emptyList()
        return runCatching {
            val arr = JSONArray(raw)
            (0 until arr.length()).mapNotNull { i -> rowFromJson(arr.getJSONObject(i)) }
        }.getOrDefault(emptyList())
    }

    private fun writeRows(ctx: Context, rows: List<Row>) {
        val arr = JSONArray()
        rows.forEach { arr.put(rowToJson(it)) }
        Auth.securePrefs(ctx).edit()
            .putString(KEY_CUSTOM_FOODS, arr.toString())
            .apply()
    }

    private fun rowToJson(row: Row): JSONObject {
        val f = row.food
        val obj = JSONObject()
            .put("id", row.id)
            .put("name", f.name)
            .put("source", f.source)
            .put("description", f.description)
            .put("per100g", nutritionToJson(f.per100g))
        if (!f.brand.isNullOrBlank()) obj.put("brand", f.brand)
        return obj
    }

    private fun rowFromJson(obj: JSONObject): Row? = runCatching {
        val id = obj.optString("id").takeIf { it.isNotEmpty() } ?: mintId()
        val name = obj.optString("name").takeIf { it.isNotEmpty() } ?: return@runCatching null
        val brand = obj.optString("brand").takeIf { it.isNotEmpty() }
        val source = obj.optString("source").takeIf { it.isNotEmpty() } ?: "user-created"
        val description = obj.optString("description")
        val per100g = nutritionFromJson(obj.optJSONObject("per100g"))
        Row(
            id = id,
            food = FoodItem(
                name = name,
                brand = brand,
                source = source,
                description = description,
                per100g = per100g,
            ),
        )
    }.getOrNull()

    private fun nutritionToJson(n: NutritionFacts): JSONObject = JSONObject()
        .put("kcal", n.kcal)
        .put("carbsG", n.carbsG)
        .put("proteinG", n.proteinG)
        .put("fatG", n.fatG)
        .put("sugarG", n.sugarG)

    private fun nutritionFromJson(obj: JSONObject?): NutritionFacts {
        if (obj == null) return NutritionFacts(kcal = 0, carbsG = 0.0, proteinG = 0.0, fatG = 0.0, sugarG = 0.0)
        return NutritionFacts(
            kcal = obj.optInt("kcal", 0),
            carbsG = obj.optDouble("carbsG", 0.0),
            proteinG = obj.optDouble("proteinG", 0.0),
            fatG = obj.optDouble("fatG", 0.0),
            sugarG = obj.optDouble("sugarG", 0.0),
        )
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
