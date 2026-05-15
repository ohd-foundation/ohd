"""OpenFoodFacts (OFF) resolver — used by the MCP food-resolution tools.

Mirrors the proxy-first / direct-fallback behaviour of the Android
``OpenFoodFacts.kt`` resolver (``connect/android/.../data/OpenFoodFacts.kt``)
so an LLM agent can chain ``food_lookup_barcode`` / ``food_search`` into
``log_food`` when the user names a product without giving full nutrition.

Order of resolution:

1. OHD SaaS proxy at ``https://api.ohd.dev/v1/openfoodfacts`` (short timeout;
   not deployed today, fails fast).
2. Public OFF v2 / search-CGI endpoints directly.

Returns dicts (not pydantic models) so the MCP tool surface stays JSON-native;
the LLM is the consumer and reads field names directly.
"""

from __future__ import annotations

from typing import Any
from urllib.parse import quote

import httpx

# OFF asks API consumers to identify themselves in User-Agent.
USER_AGENT = "OHD-Connect-MCP/0.1 (+https://ohd.dev)"

PROXY_PRODUCT = "https://api.ohd.dev/v1/openfoodfacts"
PROXY_SEARCH = "https://api.ohd.dev/v1/openfoodfacts/search"

OFF_PRODUCT = "https://world.openfoodfacts.org/api/v2/product"
OFF_SEARCH = "https://world.openfoodfacts.org/cgi/search.pl"

# Intentionally no ``fields=`` selector. Early prototypes used one with
# ``fields=...nutriments,nutriments_100g,...`` — the bogus ``nutriments_100g``
# name made OFF substitute ``nutrition_data*`` markers for the whole nested
# ``nutriments`` object, zeroing every macro on every product. Pulling the
# full payload also matches the user's intent (E-numbers, caffeine, vitamins,
# NOVA / Nutri-Score, ingredient list — everything goes into food.eaten).

# Proxy timeouts are short — falling through fast is the right behaviour
# while api.ohd.dev hasn't shipped. OFF direct gets more headroom.
PROXY_CONNECT_S = 2.0
PROXY_READ_S = 3.0
OFF_CONNECT_S = 4.0
OFF_READ_S = 8.0


async def lookup_barcode(barcode: str) -> dict[str, Any] | None:
    """Resolve an EAN/UPC to a product dict, or ``None`` if neither endpoint knows it."""
    cleaned = (barcode or "").strip()
    if not cleaned.isdigit() or not (6 <= len(cleaned) <= 14):
        return None

    proxy_url = f"{PROXY_PRODUCT}/{cleaned}"
    body = await _http_get(proxy_url, PROXY_CONNECT_S, PROXY_READ_S)
    if body is not None:
        product = _parse_product(body, source="OpenFoodFacts (via OHD proxy)")
        if product is not None:
            return product

    off_url = f"{OFF_PRODUCT}/{cleaned}.json"
    body = await _http_get(off_url, OFF_CONNECT_S, OFF_READ_S)
    if body is None:
        return None
    return _parse_product(body, source="OpenFoodFacts")


async def search(query: str, page_size: int = 10) -> list[dict[str, Any]]:
    """Free-text product search. Empty list when both endpoints fail or no hits."""
    cleaned = (query or "").strip()
    if not cleaned:
        return []
    page_size = max(1, min(page_size, 50))

    encoded = quote(cleaned, safe="")
    proxy_url = f"{PROXY_SEARCH}?query={encoded}&page_size={page_size}"
    body = await _http_get(proxy_url, PROXY_CONNECT_S, PROXY_READ_S)
    if body is not None:
        hits = _parse_search(body, source="OpenFoodFacts (via OHD proxy)")
        if hits:
            return hits

    off_url = (
        f"{OFF_SEARCH}?search_terms={encoded}&search_simple=1"
        f"&action=process&json=1&page_size={page_size}"
    )
    body = await _http_get(off_url, OFF_CONNECT_S, OFF_READ_S)
    if body is None:
        return []
    return _parse_search(body, source="OpenFoodFacts")


async def _http_get(url: str, connect_s: float, read_s: float) -> dict[str, Any] | None:
    timeout = httpx.Timeout(connect=connect_s, read=read_s, write=read_s, pool=read_s)
    headers = {"User-Agent": USER_AGENT, "Accept": "application/json"}
    try:
        async with httpx.AsyncClient(timeout=timeout, follow_redirects=True) as client:
            response = await client.get(url, headers=headers)
    except (httpx.HTTPError, OSError):
        return None
    if response.status_code < 200 or response.status_code >= 300:
        return None
    try:
        return response.json()
    except ValueError:
        return None


def _parse_product(body: dict[str, Any], source: str) -> dict[str, Any] | None:
    """OFF v2 product GET → flat dict. Returns ``None`` if status≠1 or no product."""
    status = body.get("status")
    if status != 1:
        return None
    product = body.get("product")
    if not isinstance(product, dict):
        return None
    return _map_product(product, source=source)


def _parse_search(body: dict[str, Any], source: str) -> list[dict[str, Any]]:
    products = body.get("products")
    if not isinstance(products, list):
        return []
    out: list[dict[str, Any]] = []
    for entry in products:
        if not isinstance(entry, dict):
            continue
        name = (entry.get("product_name") or "").strip()
        brand = (entry.get("brands") or "").strip()
        if not name and not brand:
            continue
        out.append(_map_product(entry, source=source))
    return out


def _map_product(product: dict[str, Any], source: str) -> dict[str, Any]:
    name_raw = (product.get("product_name") or "").strip()
    brands_raw = (product.get("brands") or "").strip()
    code = (product.get("code") or "").strip()

    if name_raw:
        name = name_raw
    elif brands_raw:
        name = brands_raw
    else:
        name = f"Unknown product (barcode {code})" if code else "Unknown product"

    brand = brands_raw.split(",", 1)[0].strip() if brands_raw else None

    nutriments = product.get("nutriments")
    if not isinstance(nutriments, dict):
        nutriments = {}

    # Sodium: prefer `sodium_100g` (grams → mg), else approximate from salt (÷2.5).
    sodium_g = _f(nutriments, "sodium_100g")
    salt_g = _f(nutriments, "salt_100g")
    if sodium_g > 0:
        sodium_mg: float = sodium_g * 1_000.0
    elif salt_g > 0:
        sodium_mg = (salt_g / 2.5) * 1_000.0
    else:
        sodium_mg = 0.0

    per_100g = {
        "kcal": int(_f(nutriments, "energy-kcal_100g")),
        "carbs_g": _f(nutriments, "carbohydrates_100g"),
        "protein_g": _f(nutriments, "proteins_100g"),
        "fat_g": _f(nutriments, "fat_100g"),
        "sugar_g": _f(nutriments, "sugars_100g"),
        "fiber_g": _f(nutriments, "fiber_100g"),
        "saturated_fat_g": _f(nutriments, "saturated-fat_100g"),
        "trans_fat_g": _f(nutriments, "trans-fat_100g"),
        "sodium_mg": sodium_mg,
        "cholesterol_mg": _f(nutriments, "cholesterol_100g") * 1_000.0,
        "potassium_mg": _f(nutriments, "potassium_100g") * 1_000.0,
        "calcium_mg": _f(nutriments, "calcium_100g") * 1_000.0,
        "iron_mg": _f(nutriments, "iron_100g") * 1_000.0,
        "vitamin_c_mg": _f(nutriments, "vitamin-c_100g") * 1_000.0,
        "vitamin_d_mcg": _f(nutriments, "vitamin-d_100g") * 1_000_000.0,
        "caffeine_mg": _f(nutriments, "caffeine_100g") * 1_000.0,
    }

    nova = product.get("nova_group")
    nova_int: int | None
    if isinstance(nova, int) and 1 <= nova <= 4:
        nova_int = nova
    elif isinstance(nova, str):
        try:
            parsed = int(nova)
            nova_int = parsed if 1 <= parsed <= 4 else None
        except ValueError:
            nova_int = None
    else:
        nova_int = None

    nutri = (product.get("nutriscore_grade") or "").strip().lower() or None
    if nutri in {"unknown", "not-applicable"}:
        nutri = None
    eco = (product.get("ecoscore_grade") or "").strip().lower() or None
    if eco in {"unknown", "not-applicable"}:
        eco = None

    return {
        "barcode": code or None,
        "name": name,
        "brand": brand,
        "source": source,
        "description": _describe(product),
        "package_serving": _package_serving(product),
        "per_100g": per_100g,
        "image_thumb_url": (product.get("image_thumb_url") or None),
        # Composition — every E-number, every allergen, every analysis tag.
        # Empty lists when OFF didn't have data; never None so JSON consumers
        # can rely on the shape.
        "additives": _strip_tags(product.get("additives_tags")),
        "allergens": _strip_tags(product.get("allergens_tags")),
        "traces": _strip_tags(product.get("traces_tags")),
        "ingredients": _strip_tags(product.get("ingredients_tags"), limit=50),
        "ingredients_analysis": _strip_tags(product.get("ingredients_analysis_tags")),
        "labels": _strip_tags(product.get("labels_tags"), limit=20),
        "nova_group": nova_int,
        "nutri_score": nutri,
        "eco_score": eco,
        # Every ``*_100g`` nutriment OFF returned, in OFF's native units. The
        # LLM picks what to surface — caffeine, omega-3, phosphorus, etc.
        "all_nutriments_per_100g": _harvest_all(nutriments),
    }


def _harvest_all(nutriments: dict[str, Any]) -> dict[str, float]:
    """Snapshot every ``*_100g`` nutriment, stripping the suffix."""
    out: dict[str, float] = {}
    for key in nutriments:
        if not isinstance(key, str) or not key.endswith("_100g"):
            continue
        if key == "energy_100g" or key.startswith("nutrition-score"):
            continue
        value = _f(nutriments, key)
        if value == 0.0:
            continue
        out[key.removesuffix("_100g")] = value
    return out


def _strip_tags(tags: Any, limit: int | None = None) -> list[str]:
    """OFF tag arrays → plain lowercase slugs, dropping the ``en:`` prefix."""
    if not isinstance(tags, list):
        return []
    out: list[str] = []
    for tag in tags:
        if not isinstance(tag, str) or not tag:
            continue
        idx = tag.find(":")
        out.append(tag[idx + 1:] if 0 < idx <= 3 else tag)
        if limit is not None and len(out) >= limit:
            break
    return out


def _describe(product: dict[str, Any]) -> str:
    generic = (product.get("generic_name") or "").strip()
    if generic:
        return generic
    tags = product.get("categories_tags")
    if not isinstance(tags, list):
        return ""
    cleaned: list[str] = []
    for tag in tags:
        if not isinstance(tag, str) or not tag:
            continue
        # Drop language prefix like "en:" → keep slug.
        idx = tag.find(":")
        cleaned.append(tag[idx + 1:] if 0 < idx <= 3 else tag)
    return ", ".join(cleaned[-3:])


def _package_serving(product: dict[str, Any]) -> dict[str, Any] | None:
    label = (product.get("quantity") or "").strip()
    raw = product.get("product_quantity")
    grams: float | None
    if raw is None:
        grams = None
    elif isinstance(raw, (int, float)):
        grams = float(raw)
    elif isinstance(raw, str):
        stripped = raw.strip()
        try:
            grams = float(stripped) if stripped else None
        except ValueError:
            grams = None
    else:
        grams = None
    if not label and grams is None:
        return None
    if not label and grams is not None:
        label = f"{int(grams)} g"
    return {"name": label, "grams": grams or 0.0}


def _f(obj: dict[str, Any], key: str) -> float:
    """Like ``JSONObject.optDouble`` — missing/non-numeric → 0.0."""
    v = obj.get(key)
    if isinstance(v, (int, float)):
        return float(v)
    if isinstance(v, str):
        try:
            return float(v)
        except ValueError:
            return 0.0
    return 0.0
