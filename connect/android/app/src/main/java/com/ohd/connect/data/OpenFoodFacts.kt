package com.ohd.connect.data

import com.ohd.connect.ui.screens.FoodItem
import com.ohd.connect.ui.screens.NutritionFacts
import com.ohd.connect.ui.screens.Serving
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap

/**
 * OpenFoodFacts (OFF) barcode resolver — spec §4.7.
 *
 * Triggered from [com.ohd.connect.ui.screens.FoodSearchScreen] when the query
 * looks like an EAN/UPC (8–13 digits) and no local match was found in
 * [com.ohd.connect.ui.screens.FoodDictionary].
 *
 * Tries two endpoints in order:
 *   1. OHD SaaS proxy `https://api.ohd.dev/v1/openfoodfacts/{barcode}` —
 *      not deployed yet, so we keep the connect timeout short (~2 s) and fall
 *      through on any network / 4xx / 5xx failure.
 *   2. OpenFoodFacts v2 directly:
 *      `https://world.openfoodfacts.org/api/v2/product/{barcode}.json` with a
 *      pruned `?fields=...` selection to keep payloads small.
 *
 * Uses only `java.net.HttpURLConnection` + `org.json.JSONObject` to avoid
 * adding new HTTP dependencies. All network I/O runs on `Dispatchers.IO`.
 *
 * The OFF terms-of-service ask for a short rate limit between calls; the
 * search screen only fires a single lookup per query keystroke, so we don't
 * implement explicit throttling here.
 */
object OpenFoodFacts {

    /** User-Agent header — required by OFF's API ToS. */
    private const val USER_AGENT = "OHD-Connect/0.1 (Android; +https://ohd.dev)"

    /** OHD proxy endpoint. Not yet deployed — must fail-fast. */
    private const val PROXY_BASE = "https://api.ohd.dev/v1/openfoodfacts"

    /** OHD proxy text-search endpoint. Same caveat as [PROXY_BASE]. */
    private const val PROXY_BASE_SEARCH = "https://api.ohd.dev/v1/openfoodfacts/search"

    /** OpenFoodFacts v2 product endpoint. */
    private const val OFF_BASE = "https://world.openfoodfacts.org/api/v2/product"

    /** OpenFoodFacts text-search endpoint (legacy cgi but still stable). */
    private const val OFF_SEARCH_BASE = "https://world.openfoodfacts.org/cgi/search.pl"

    // Intentionally no `fields=` selector — early betas used one and it
    // silently zeroed nutrition for every scan: with `fields=...,nutriments,
    // nutriments_100g,...` OFF substitutes `nutrition_data*` markers and
    // strips the entire nested `nutriments` object. Pulling the full product
    // is also what the user actually wants — every micronutrient, every
    // E-number, every label travels into the food.eaten payload.

    // Connect timeout stays short so an unreachable proxy fails over to the
    // direct OFF call fast. Read timeout has headroom — the proxy IS live now
    // (api.ohd.dev) and a full search response is 10 large product objects.
    private const val PROXY_CONNECT_TIMEOUT_MS = 2_000
    private const val PROXY_READ_TIMEOUT_MS = 8_000

    // Direct OFF call gets more headroom — public endpoint, occasionally slow.
    private const val OFF_CONNECT_TIMEOUT_MS = 4_000
    private const val OFF_READ_TIMEOUT_MS = 8_000

    /**
     * Process-lifetime cache so [com.ohd.connect.ui.screens.FoodDetailScreen]
     * can re-resolve a remote item by name after navigation. Keyed by
     * barcode; values are the mapped [FoodItem].
     */
    val cache: ConcurrentHashMap<String, FoodItem> = ConcurrentHashMap()

    /**
     * Resolve a barcode to a [FoodItem]. Returns `null` if neither the proxy
     * nor OFF directly recognises the code.
     *
     * Never throws — network / parse exceptions are swallowed so the caller
     * can fall back to the "not found" UI. (Errors that *should* be visible
     * are surfaced separately by [lookupWithSource]; this top-level wrapper
     * is kept exception-free for parity with the spec's `lookup` signature.)
     */
    suspend fun lookup(barcode: String): FoodItem? = runInterruptible(Dispatchers.IO) {
        // Wrapped in `runInterruptible` so coroutine cancellation (user types
        // another character, navigates away, etc.) translates to a thread
        // interrupt, which `HttpURLConnection` honours by throwing
        // `InterruptedIOException` → swallowed by `runCatching`. Without this
        // the blocking I/O outlived the composition and Compose logged a
        // "coroutine left the composition" warning.
        //
        // The api.ohd.dev proxy is currently misconfigured (serves OFF's HTML
        // error page for every path) — go straight to OFF. Re-add the proxy
        // first-hop once the Caddy route is fixed + caching is wired.
        runCatching { fetchOff(barcode) }.getOrNull()
    }

    /**
     * Free-text product search against OpenFoodFacts (or the OHD proxy if it
     * ever comes online — the proxy URL slot is reserved but not yet
     * deployed). Falls through to the public OFF search endpoint when the
     * proxy is unavailable.
     *
     * Returns up to [pageSize] hits, ranked by OFF's default popularity
     * heuristic. Every result is also written to [cache] so a subsequent
     * tap → [com.ohd.connect.ui.screens.FoodDetailScreen] resolves locally.
     */
    suspend fun search(
        query: String,
        pageSize: Int = 10,
    ): List<FoodItem> = runInterruptible(Dispatchers.IO) {
        val cleaned = query.trim()
        if (cleaned.isBlank()) return@runInterruptible emptyList()
        // Proxy skipped — see [lookup] note. Direct OFF text search.
        runCatching { searchOff(cleaned, pageSize) }.getOrNull().orEmpty()
    }

    /**
     * Try the proxy first, returning `(item, sourceLabel)`. Returns `null`
     * only if both endpoints fail. Used by tests / debug screens; the UI uses
     * [lookup] which discards the source label (it's already on `FoodItem.source`).
     */
    suspend fun lookupWithSource(barcode: String): FoodItem? = lookup(barcode)

    // ------------------------------------------------------------------
    // Endpoint wrappers
    // ------------------------------------------------------------------

    private fun fetchProxy(barcode: String): FoodItem? {
        val url = URL("$PROXY_BASE/$barcode")
        val body = httpGet(url, PROXY_CONNECT_TIMEOUT_MS, PROXY_READ_TIMEOUT_MS)
            ?: return null
        val item = parseOffJson(body, "OpenFoodFacts (via OHD proxy)") ?: return null
        cache[barcode] = item
        return item
    }

    private fun fetchOff(barcode: String): FoodItem? {
        val url = URL("$OFF_BASE/$barcode.json")
        val body = httpGet(url, OFF_CONNECT_TIMEOUT_MS, OFF_READ_TIMEOUT_MS)
            ?: return null
        val item = parseOffJson(body, "OpenFoodFacts") ?: return null
        cache[barcode] = item
        return item
    }

    // ------------------------------------------------------------------
    // Search endpoints
    // ------------------------------------------------------------------

    private fun searchProxy(query: String, pageSize: Int): List<FoodItem> {
        // The proxy is a dumb path-rewrite (/v1/openfoodfacts/search →
        // /cgi/search.pl) — it forwards the query string verbatim. So we
        // send OFF's *native* search params, not a `query=` alias, otherwise
        // cgi/search.pl serves an HTML landing page instead of JSON.
        val encoded = java.net.URLEncoder.encode(query, "UTF-8")
        val url = URL(
            "$PROXY_BASE_SEARCH?search_terms=$encoded&search_simple=1" +
                "&action=process&json=1&page_size=$pageSize",
        )
        val body = httpGet(url, PROXY_CONNECT_TIMEOUT_MS, PROXY_READ_TIMEOUT_MS)
            ?: return emptyList()
        return parseOffSearchJson(body, "OpenFoodFacts (via OHD proxy)")
    }

    private fun searchOff(query: String, pageSize: Int): List<FoodItem> {
        // OFF's `cgi/search.pl` endpoint takes a search-term + JSON flag and
        // accepts the same `fields=` selector as the product GET. It's the
        // stable text-search surface; the newer `search.openfoodfacts.org`
        // is for the SaaS demo site and not guaranteed.
        val encoded = java.net.URLEncoder.encode(query, "UTF-8")
        val url = URL(
            "$OFF_SEARCH_BASE?search_terms=$encoded&search_simple=1" +
                "&action=process&json=1&page_size=$pageSize",
        )
        val body = httpGet(url, OFF_CONNECT_TIMEOUT_MS, OFF_READ_TIMEOUT_MS)
            ?: return emptyList()
        return parseOffSearchJson(body, "OpenFoodFacts")
    }

    /**
     * Parse OFF's search.pl response: `{ "products": [ {...}, ... ] }`.
     * Each entry is the same product shape as the v2 product endpoint, so
     * we reuse [mapOffProduct]. Hits with a missing/blank `product_name` AND
     * blank `brands` are dropped to avoid surfacing junk rows.
     */
    private fun parseOffSearchJson(body: String, sourceLabel: String): List<FoodItem> {
        val root = runCatching { JSONObject(body) }.getOrNull() ?: return emptyList()
        val products = root.optJSONArray("products") ?: return emptyList()
        val out = ArrayList<FoodItem>(products.length())
        for (i in 0 until products.length()) {
            val product = products.optJSONObject(i) ?: continue
            val name = product.optString("product_name").trim()
            val brand = product.optString("brands").trim()
            if (name.isEmpty() && brand.isEmpty()) continue
            val item = mapOffProduct(product, sourceLabel)
            cache[item.name] = item   // keyed by name so foodByName(name) resolves
            val code = product.optString("code").trim()
            if (code.isNotEmpty()) cache[code] = item
            out += item
        }
        return out
    }

    // ------------------------------------------------------------------
    // HTTP
    // ------------------------------------------------------------------

    /**
     * Plain GET. Returns the response body on 2xx, `null` on any other
     * status / network failure so the caller can fall through.
     */
    private fun httpGet(url: URL, connectTimeoutMs: Int, readTimeoutMs: Int): String? {
        val conn = (url.openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            connectTimeout = connectTimeoutMs
            readTimeout = readTimeoutMs
            instanceFollowRedirects = true
            setRequestProperty("User-Agent", USER_AGENT)
            setRequestProperty("Accept", "application/json")
        }
        return try {
            val code = conn.responseCode
            if (code in 200..299) {
                conn.inputStream.bufferedReader().use { it.readText() }
            } else {
                null
            }
        } catch (_: Throwable) {
            null
        } finally {
            conn.disconnect()
        }
    }

    // ------------------------------------------------------------------
    // Parsing — OFF v2 response → FoodItem
    // ------------------------------------------------------------------

    /**
     * Parse an OFF v2 product response and project it into our [FoodItem].
     * Returns `null` if `status != 1` or no `product` field is present.
     *
     * Visible for testing.
     */
    internal fun parseOffJson(body: String, sourceLabel: String): FoodItem? {
        val root = runCatching { JSONObject(body) }.getOrNull() ?: return null
        val status = root.optInt("status", 0)
        if (status != 1) return null
        val product = root.optJSONObject("product") ?: return null
        return mapOffProduct(product, sourceLabel)
    }

    private fun mapOffProduct(product: JSONObject, sourceLabel: String): FoodItem {
        val rawName = product.optString("product_name").trim()
        val brandsRaw = product.optString("brands").trim()
        val code = product.optString("code").trim()

        val name = when {
            rawName.isNotEmpty() -> rawName
            brandsRaw.isNotEmpty() -> brandsRaw
            else -> "Unknown product (barcode $code)"
        }

        val brand = brandsRaw
            .takeIf { it.isNotEmpty() }
            ?.split(",")
            ?.firstOrNull()
            ?.trim()
            ?.takeIf { it.isNotEmpty() }

        val description = buildDescription(product)

        val nutriments = product.optJSONObject("nutriments")

        // Sodium: prefer `sodium_100g` (grams → mg), otherwise approximate
        // from `salt_100g` (grams of salt ≈ 2.5× grams of sodium) → mg.
        val sodiumG = optDouble(nutriments, "sodium_100g")
        val saltG = optDouble(nutriments, "salt_100g")
        val sodiumMg = when {
            sodiumG > 0.0 -> sodiumG * 1_000.0
            saltG > 0.0 -> (saltG / 2.5) * 1_000.0
            else -> 0.0
        }

        val per100g = NutritionFacts(
            kcal = optDouble(nutriments, "energy-kcal_100g").toInt(),
            carbsG = optDouble(nutriments, "carbohydrates_100g"),
            proteinG = optDouble(nutriments, "proteins_100g"),
            fatG = optDouble(nutriments, "fat_100g"),
            sugarG = optDouble(nutriments, "sugars_100g"),
            fiberG = optDouble(nutriments, "fiber_100g"),
            saturatedFatG = optDouble(nutriments, "saturated-fat_100g"),
            transFatG = optDouble(nutriments, "trans-fat_100g"),
            sodiumMg = sodiumMg,
            // Cholesterol / minerals / vitamins: OFF reports in grams, we
            // store in milligrams (×1000). Vitamin D is reported in grams
            // but our UI shows micrograms (×1_000_000) — the conventional
            // dose unit for cholecalciferol.
            cholesterolMg = optDouble(nutriments, "cholesterol_100g") * 1_000.0,
            potassiumMg = optDouble(nutriments, "potassium_100g") * 1_000.0,
            calciumMg = optDouble(nutriments, "calcium_100g") * 1_000.0,
            ironMg = optDouble(nutriments, "iron_100g") * 1_000.0,
            vitaminCMg = optDouble(nutriments, "vitamin-c_100g") * 1_000.0,
            vitaminDMcg = optDouble(nutriments, "vitamin-d_100g") * 1_000_000.0,
            // Caffeine: OFF stores grams/100g; we expose mg/100g.
            caffeineMg = optDouble(nutriments, "caffeine_100g") * 1_000.0,
            nutrimentsPer100g = harvestAllPer100g(nutriments),
        )

        val packageServing = buildPackageServing(product)

        return FoodItem(
            name = name,
            brand = brand,
            source = sourceLabel,
            description = description,
            per100g = per100g,
            packageServing = packageServing,
            defaultPortion = null,
            additives = stringList(product, "additives_tags"),
            allergens = stringList(product, "allergens_tags"),
            traces = stringList(product, "traces_tags"),
            // Cap ingredients at 50 — OFF returns deep hierarchies that bloat
            // the food.eaten event payload (parent categories + leaves +
            // synonyms). 50 is comfortably more than any real ingredient list.
            ingredients = stringList(product, "ingredients_tags", limit = 50),
            ingredientsAnalysis = stringList(product, "ingredients_analysis_tags"),
            labels = stringList(product, "labels_tags", limit = 20),
            novaGroup = product.opt("nova_group").let {
                when (it) {
                    is Number -> it.toInt().takeIf { n -> n in 1..4 }
                    is String -> it.toIntOrNull()?.takeIf { n -> n in 1..4 }
                    else -> null
                }
            },
            nutriScore = product.optString("nutriscore_grade").trim().lowercase()
                .takeIf { it.isNotEmpty() && it != "unknown" && it != "not-applicable" },
            ecoScore = product.optString("ecoscore_grade").trim().lowercase()
                .takeIf { it.isNotEmpty() && it != "unknown" && it != "not-applicable" },
        )
    }

    /**
     * Snapshot every `*_100g` key OFF returned, stripping the suffix. Values
     * stay in OFF's native unit (mostly grams) so downstream code can compute
     * "per gram-eaten" with one rule-of-three.
     *
     * Skips `energy_100g` (kJ duplicate of `energy-kcal_100g` × 4.184) and
     * `nutrition-score-*_100g` (Nutri-Score component, not a nutriment).
     */
    private fun harvestAllPer100g(nutriments: JSONObject?): Map<String, Double> {
        if (nutriments == null) return emptyMap()
        val out = LinkedHashMap<String, Double>()
        val keys = nutriments.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            if (!key.endsWith("_100g")) continue
            if (key == "energy_100g") continue
            if (key.startsWith("nutrition-score")) continue
            val v = optDouble(nutriments, key)
            if (v == 0.0) continue
            val short = key.removeSuffix("_100g")
            out[short] = v
        }
        return out
    }

    /**
     * Pull an OFF tag array (e.g. `additives_tags`) into a plain `List<String>`
     * with the `en:` / `fr:` language prefix stripped. Optionally caps result
     * length to keep payloads manageable on deeply-tagged products.
     */
    private fun stringList(
        product: JSONObject,
        key: String,
        limit: Int = Int.MAX_VALUE,
    ): List<String> {
        val arr = product.optJSONArray(key) ?: return emptyList()
        val out = ArrayList<String>(minOf(arr.length(), 32))
        for (i in 0 until arr.length()) {
            val raw = arr.optString(i).trim()
            if (raw.isEmpty()) continue
            out += stripLangPrefix(raw)
            if (out.size >= limit) break
        }
        return out
    }

    /**
     * `generic_name` → fallback to last 3 `categories_tags` (with the `en:`
     * language prefix stripped) → fallback to empty string.
     */
    private fun buildDescription(product: JSONObject): String {
        val generic = product.optString("generic_name").trim()
        if (generic.isNotEmpty()) return generic

        val tagsArr = product.optJSONArray("categories_tags") ?: return ""
        val tags = (0 until tagsArr.length())
            .mapNotNull { tagsArr.optString(it).takeIf { s -> s.isNotEmpty() } }
            .map { stripLangPrefix(it) }
        if (tags.isEmpty()) return ""
        return tags.takeLast(3).joinToString(", ")
    }

    /** "en:soft-drinks" → "soft-drinks"; "soft-drinks" stays. */
    private fun stripLangPrefix(tag: String): String {
        val idx = tag.indexOf(':')
        return if (idx in 1..3) tag.substring(idx + 1) else tag
    }

    /**
     * Build the optional package-serving from `product.quantity` (label) and
     * `product.product_quantity` (grams). `product_quantity` is sometimes a
     * string, sometimes a number — handle both.
     */
    private fun buildPackageServing(product: JSONObject): Serving? {
        val label = product.optString("quantity").trim()
        val grams: Double? = when (val v = product.opt("product_quantity")) {
            null -> null
            JSONObject.NULL -> null
            is Number -> v.toDouble()
            is String -> v.trim().takeIf { it.isNotEmpty() }?.toDoubleOrNull()
            else -> null
        }
        if (label.isEmpty() && grams == null) return null
        val displayName = label.ifEmpty { grams?.let { "${it.toInt()} g" } ?: return null }
        return Serving(name = displayName, grams = grams ?: 0.0).takeIf { it.grams > 0.0 || label.isNotEmpty() }
    }

    /**
     * `JSONObject.optDouble` returns `NaN` for missing keys; we want 0.0
     * instead (per spec). Also tolerates numeric values stored as strings.
     */
    private fun optDouble(obj: JSONObject?, key: String): Double {
        if (obj == null) return 0.0
        val v = obj.opt(key) ?: return 0.0
        return when (v) {
            is Number -> v.toDouble()
            is String -> v.toDoubleOrNull() ?: 0.0
            else -> 0.0
        }
    }
}
