# Research: OpenFoodFacts Integration

> How we get nutrition data into OHD from a scanned barcode.

## What OpenFoodFacts is

A collaborative, free, and open database of ingredients, nutrition facts, and information on food products from around the world. It's maintained by a non-profit association of volunteers and has over 4 million products from 150 countries, contributed by 100,000+ individuals via the Android and iPhone apps. The data is openly licensed (Open Database License for the database; Database Contents License for individual entries; CC-BY-SA for product images).

Because the database is crowd-sourced and volunteer-maintained, accuracy is not guaranteed. The user assumes the entire risk of using the data. We'll surface this to users ("nutrition data from OpenFoodFacts, may be inaccurate") rather than pretending the numbers are clinical-grade.

## API basics

Current stable version: **v2**. Version 3 is in active development and may change.

### Base URL

- **Production:** `https://world.openfoodfacts.org/` (or `.net` for newer deployments)
- **Staging:** `https://world.openfoodfacts.net/` (requires HTTP Basic Auth `off:off`)

For MVP, use production read-only. For any writes, start with staging.

### Core endpoint: barcode lookup

```
GET https://world.openfoodfacts.org/api/v2/product/{barcode}
```

Example:

```
GET https://world.openfoodfacts.org/api/v2/product/3017624010701
```

Returns everything known about the product. Response shape:

```json
{
  "code": "3017624010701",
  "product": {
    "product_name": "Nutella",
    "brands": "Ferrero",
    "quantity": "400 g",
    "serving_size": "15 g",
    "nutriments": { ... },
    "nutriscore_grade": "e",
    "nova_group": 4,
    "ecoscore_grade": "d",
    "categories": "Spreads, Sweet spreads, Cocoa and hazelnuts spreads",
    "labels": "...",
    "ingredients_text": "...",
    "allergens": "en:milk,en:nuts,en:soybeans",
    "image_url": "https://..."
  },
  "status": 1,
  "status_verbose": "product found"
}
```

### Field selection

The full response is large. Use the `fields` query parameter to limit it:

```
GET /api/v2/product/3017624010701?fields=product_name,brands,quantity,serving_size,nutriments,image_url,allergens,ingredients_text
```

For OHD's food logging UI, we need:
- `product_name`
- `brands`
- `quantity` (package size, for reference)
- `serving_size` (default portion)
- `nutriments` (all the values we store)
- `image_url` (for visual confirmation)
- `allergens`
- `ingredients_text`

### Nutriments object

The `nutriments` object has both absolute values (`_100g`, `_serving`) and source values. Fields that end with `_100g` correspond to the amount of a nutriment (in g, or kJ for energy) for 100 g or 100 ml of product. Fields that end with `_serving` correspond to the amount of a nutriment for 1 serving of the product. Fields that end with `_value` correspond to the amount in the unit provided by `_unit`.

The fields we care about per nutriment (generic pattern):
- `energy-kcal_100g`, `energy-kcal_serving`
- `fat_100g`, `fat_serving`
- `saturated-fat_100g`
- `carbohydrates_100g`
- `sugars_100g`
- `fiber_100g`
- `proteins_100g`
- `salt_100g` or `sodium_100g`

We'll store nutrition in OHD as absolute values for the quantity consumed (not per 100g). The calculation is straightforward: `per_100g_value * (quantity_g / 100)`.

### Search (not critical for MVP)

For manual text search (when user doesn't have a barcode):

```
GET /cgi/search.pl?search_terms=banana&search_simple=1&action=process&json=1
```

Returns paginated results. Rate limited to **10 req/min**. Not suitable for search-as-you-type.

## Rate limits

From the OpenFoodFacts docs:

- **Product read queries** (what we mostly do): no specified limit as long as usage is reasonable; they call out "1 API call = 1 real scan by a user" as the expected pattern. Bulk scraping gets IP-banned.
- **Search queries** (`/cgi/search.pl` or `/api/v*/search`): **10 req/min**. Do not use for search-as-you-type.
- **Facet queries** (`/categories`, `/label/organic`, etc.): **2 req/min**.

If usage comes from users directly (e.g., mobile app), rate limits apply per user, not per IP.

**Our strategy:**
- Barcode scans are 1:1 with user actions, well within limits.
- Text search is used only when the user deliberately initiates it (tap a "Search" button), not as they type.
- If we ever need bulk data, use their nightly database dump (MongoDB, CSV, or JSONL — whole database exports available from https://world.openfoodfacts.org/data), not the API.

## User-Agent and identification

The OpenFoodFacts team asks that API clients send a descriptive User-Agent header so they can contact the app if there are issues:

```
User-Agent: OHDC-Android/0.1.0 (contact@ohd.org)
```

For the Android app, we'll set this in our Retrofit OkHttp client.

## Caching

Product data rarely changes. Aggressive client-side caching is appropriate:

- Cache successful barcode lookups indefinitely (keyed by barcode) in local storage.
- Cache negative lookups (barcode not found) for 24 hours — so if a product gets added to OpenFoodFacts later, the user will eventually see it.
- On a cache hit, skip the API call entirely.

On the server side, the OHD backend could also maintain a shared cache (Redis), so multiple users scanning the same barcode don't each hit OpenFoodFacts. This is especially useful for a SaaS deployment. MVP: client-side only.

## OHD event mapping

A food log event looks like:

```json
{
  "event_type": "meal",
  "timestamp": "2025-01-15T18:00:00Z",
  "duration_seconds": 2700,
  "data": {
    "items": [
      {
        "openfoodfacts_barcode": "3017624010701",
        "product_name": "Nutella",
        "brands": "Ferrero",
        "quantity_grams": 30,
        "nutrition": {
          "energy_kcal": 161,
          "fat_g": 9.3,
          "saturated_fat_g": 3.2,
          "carbohydrates_g": 17.3,
          "sugars_g": 16.8,
          "fiber_g": 0,
          "proteins_g": 1.8,
          "salt_g": 0.03
        },
        "allergens": ["en:milk", "en:nuts", "en:soybeans"],
        "ingredients_text_snapshot": "..."
      }
    ],
    "total_nutrition": {
      "energy_kcal": 161,
      "fat_g": 9.3,
      "carbohydrates_g": 17.3,
      "sugars_g": 16.8,
      "proteins_g": 1.8
    },
    "notes": "with toast"
  },
  "metadata": {
    "source": "openfoodfacts",
    "openfoodfacts_fetched_at": "2025-01-15T18:00:05Z",
    "ingredient_text_snapshot_hash": "sha256:..."
  }
}
```

**Why snapshot the nutrition at log time:** OpenFoodFacts data can change (products get reformulated, contributors update values). If we always re-fetch by barcode at query time, a user's historical log could shift under them. Better to snapshot the nutrition values at log time into the event, and only reference the barcode for display/image lookup.

## Handling missing products

Sometimes a scanned barcode isn't in OpenFoodFacts. When that happens:

1. Surface a "product not found — log manually" fallback UI.
2. Let the user enter name + nutrition manually.
3. Offer to contribute the product to OpenFoodFacts (Phase 2 — write API integration).

## Handling multilingual data

OpenFoodFacts is international and has product names in multiple languages. The main `product_name` field is usually in the product's primary language; `product_name_en`, `product_name_cs`, etc. are localized variants.

For Phase 1: just use whatever `product_name` returns. Phase 2: prefer the user's locale, fall back to original.

## Write API (contributing back) — Phase 2+

When a user scans a product with missing nutrition data, we could offer to contribute. Requires an OpenFoodFacts user account (the dev account for the founder's app is already created).

```
POST https://world.openfoodfacts.net/cgi/product_jqm2.pl
    -F user_id=<our_account>
    -F password=<our_password>
    -F code=<barcode>
    -F product_name=<name>
    -F nutriment_energy-kcal=<value>
    -F nutriment_energy-kcal_unit=kcal
    ...
```

**Caveat:** writes use the production account. For legitimate user-initiated contributions, we probably want the user's own OpenFoodFacts credentials (or an OpenFoodFacts OAuth flow, once that rolls out — they're moving to Keycloak). Not MVP.

## Dependencies (client side)

### Android (Kotlin)

```kotlin
// app/build.gradle.kts
dependencies {
    implementation("com.squareup.retrofit2:retrofit:2.11.0")
    implementation("com.squareup.retrofit2:converter-moshi:2.11.0")
    implementation("com.squareup.okhttp3:okhttp:5.0.0")
    implementation("com.squareup.okhttp3:logging-interceptor:5.0.0")
}
```

```kotlin
interface OpenFoodFactsApi {
    @GET("api/v2/product/{barcode}")
    suspend fun getProduct(
        @Path("barcode") barcode: String,
        @Query("fields") fields: String = "product_name,brands,quantity,serving_size,nutriments,image_url,allergens,ingredients_text"
    ): Response<OpenFoodFactsResponse>
}

val retrofit = Retrofit.Builder()
    .baseUrl("https://world.openfoodfacts.org/")
    .client(
        OkHttpClient.Builder()
            .addInterceptor { chain ->
                chain.proceed(chain.request().newBuilder()
                    .header("User-Agent", "OHDC-Android/0.1.0 (contact@ohd.org)")
                    .build())
            }
            .build()
    )
    .addConverterFactory(MoshiConverterFactory.create())
    .build()
```

### Server side (Python) — optional

If we want server-side caching or an OHD-hosted proxy:

```python
import httpx

async def get_product(barcode: str) -> dict:
    async with httpx.AsyncClient(
        headers={"User-Agent": "OHD/0.1.0 (contact@ohd.org)"},
        timeout=10.0,
    ) as client:
        fields = ",".join([
            "product_name", "brands", "quantity", "serving_size",
            "nutriments", "image_url", "allergens", "ingredients_text"
        ])
        r = await client.get(
            f"https://world.openfoodfacts.org/api/v2/product/{barcode}",
            params={"fields": fields}
        )
        r.raise_for_status()
        return r.json()
```

## Open questions

- **Server-side proxy vs direct client call.** Probably direct from the Android app for MVP (simpler, less server load). Server-side becomes interesting when we want shared caching or when the app is offline and we want to defer lookups.
- **Localization of nutrient names.** Display "saturated fat" vs "tuk" vs "matière grasse" — display-layer concern, not storage-layer. Store in English keys; translate at display time.
- **Non-food products.** OpenFoodFacts has sibling projects: OpenProductsFacts (general), OpenBeautyFacts (cosmetics), OpenPetFoodFacts (pet food). If someone scans a cosmetic product expecting food, we should gracefully say "not a food product."
- **Generic products without barcodes.** Home-cooked meals, bulk bananas, restaurant food. Solutions: pre-defined generics from OpenFoodFacts (it has entries for "Banana", "White rice, cooked"), user-defined custom items, or future: photo-based recognition via an LLM.
