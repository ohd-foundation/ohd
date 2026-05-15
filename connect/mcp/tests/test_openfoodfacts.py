"""Tests for ``ohd_connect_mcp.openfoodfacts``.

We don't hit the network — every request is intercepted by an httpx
``MockTransport`` that lets us script per-URL responses.
"""

from __future__ import annotations

from typing import Any, Callable

import httpx
import pytest

from ohd_connect_mcp import openfoodfacts


def _redbull_payload() -> dict[str, object]:
    return {
        "status": 1,
        "product": {
            "code": "9002490100070",
            "product_name": "Red Bull Energy Drink",
            "brands": "Red Bull",
            "quantity": "250 ml",
            "product_quantity": "250",
            "generic_name": "Energy drink with taurine",
            "categories_tags": ["en:beverages", "en:soft-drinks", "en:energy-drinks"],
            "image_thumb_url": "https://images.openfoodfacts.org/red-bull.jpg",
            "nutriments": {
                "energy-kcal_100g": 45,
                "carbohydrates_100g": 11.0,
                "sugars_100g": 11.0,
                "proteins_100g": 0.4,
                "fat_100g": 0.0,
                "salt_100g": 0.1,
                "caffeine_100g": 0.032,
                "taurine_100g": 0.4,
                "vitamin-b12_100g": 0.000002,
                # Filtered out: composite energy_100g + nutrition-score component.
                "energy_100g": 190,
                "nutrition-score-fr_100g": 12,
            },
            "additives_tags": ["en:e330", "en:e150", "en:e290"],
            "allergens_tags": [],
            "traces_tags": [],
            "ingredients_tags": ["en:water", "en:sucrose", "en:caffeine"],
            "ingredients_analysis_tags": ["en:palm-oil-free", "en:maybe-vegan"],
            "labels_tags": ["en:not-advised-for-children-and-pregnant-women"],
            "nova_group": 4,
            "nutriscore_grade": "e",
            "ecoscore_grade": "not-applicable",
        },
    }


def _install_transport(
    monkeypatch: pytest.MonkeyPatch,
    handler: Callable[[httpx.Request], httpx.Response],
) -> None:
    """Replace ``httpx.AsyncClient`` so it uses our scripted transport."""
    real_async_client = httpx.AsyncClient
    transport = httpx.MockTransport(handler)

    def factory(*args: Any, **kwargs: Any) -> httpx.AsyncClient:
        kwargs.pop("timeout", None)
        return real_async_client(transport=transport, follow_redirects=True)

    monkeypatch.setattr(openfoodfacts.httpx, "AsyncClient", factory)


@pytest.mark.anyio
async def test_lookup_barcode_uses_proxy_first(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(str(request.url))
        assert "api.ohd.dev" in str(request.url)
        return httpx.Response(200, json=_redbull_payload())

    _install_transport(monkeypatch, handler)

    result = await openfoodfacts.lookup_barcode("9002490100070")
    assert result is not None
    assert result["barcode"] == "9002490100070"
    assert result["name"] == "Red Bull Energy Drink"
    assert result["brand"] == "Red Bull"
    assert result["source"] == "OpenFoodFacts (via OHD proxy)"
    assert result["per_100g"]["kcal"] == 45
    # Caffeine: 0.032 g/100g → 32 mg/100g.
    assert result["per_100g"]["caffeine_mg"] == pytest.approx(32.0)
    assert result["package_serving"] == {"name": "250 ml", "grams": 250.0}
    # E-numbers: stripped of `en:` prefix.
    assert result["additives"] == ["e330", "e150", "e290"]
    assert result["nova_group"] == 4
    assert result["nutri_score"] == "e"
    # Eco-Score "not-applicable" filtered → None.
    assert result["eco_score"] is None
    assert "palm-oil-free" in result["ingredients_analysis"]
    # all_nutriments_per_100g: caffeine + taurine + b12 + macros — but the
    # filtered `energy_100g` and `nutrition-score-fr_100g` must be excluded.
    allnutr = result["all_nutriments_per_100g"]
    assert "caffeine" in allnutr and allnutr["caffeine"] == pytest.approx(0.032)
    assert "taurine" in allnutr and allnutr["taurine"] == pytest.approx(0.4)
    assert "energy" not in allnutr
    assert "nutrition-score-fr" not in allnutr
    assert seen == ["https://api.ohd.dev/v1/openfoodfacts/9002490100070"]


@pytest.mark.anyio
async def test_lookup_barcode_falls_through_to_off(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(str(request.url))
        if "api.ohd.dev" in str(request.url):
            return httpx.Response(502)
        return httpx.Response(200, json=_redbull_payload())

    _install_transport(monkeypatch, handler)

    result = await openfoodfacts.lookup_barcode("9002490100070")
    assert result is not None
    assert result["source"] == "OpenFoodFacts"
    assert len(seen) == 2
    assert "world.openfoodfacts.org" in seen[1]


@pytest.mark.anyio
async def test_lookup_barcode_not_found(monkeypatch: pytest.MonkeyPatch) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        if "api.ohd.dev" in str(request.url):
            return httpx.Response(404)
        return httpx.Response(200, json={"status": 0})  # OFF "no product"

    _install_transport(monkeypatch, handler)

    result = await openfoodfacts.lookup_barcode("0000000000000")
    assert result is None


@pytest.mark.anyio
async def test_lookup_barcode_rejects_non_digit_input() -> None:
    # No HTTP call should happen — fast path rejects the input.
    assert await openfoodfacts.lookup_barcode("not-a-barcode") is None
    assert await openfoodfacts.lookup_barcode("") is None
    assert await openfoodfacts.lookup_barcode("12") is None  # too short


@pytest.mark.anyio
async def test_search_uses_off_when_proxy_empty(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        seen.append(url)
        if "api.ohd.dev" in url:
            return httpx.Response(200, json={"products": []})  # empty → fall through
        return httpx.Response(
            200,
            json={
                "products": [
                    _redbull_payload()["product"],
                    {"product_name": "", "brands": ""},  # filtered out
                ]
            },
        )

    _install_transport(monkeypatch, handler)

    hits = await openfoodfacts.search("red bull", page_size=5)
    assert len(hits) == 1
    assert hits[0]["name"] == "Red Bull Energy Drink"
    assert hits[0]["source"] == "OpenFoodFacts"
    assert any("search.pl" in u for u in seen)


@pytest.mark.anyio
async def test_search_blank_query_returns_empty() -> None:
    assert await openfoodfacts.search("") == []
    assert await openfoodfacts.search("   ") == []


@pytest.mark.anyio
async def test_search_clamps_page_size(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: list[httpx.URL] = []

    def handler(request: httpx.Request) -> httpx.Response:
        captured.append(request.url)
        return httpx.Response(200, json={"products": []})

    _install_transport(monkeypatch, handler)

    # The MCP tool already constrains via Pydantic (ge=1, le=50); the resolver
    # is defensive on top of that.
    await openfoodfacts.search("oats", page_size=999)
    assert all("page_size=50" in str(u) for u in captured)


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"
