"""Smoke tests for OIDC device-flow + KMS envelope round-trip.

We don't dial out — every HTTP interaction is replaced with an
``httpx.MockTransport`` so the test suite is hermetic. The KMS tests
also stay off the OS keyring (which is genuinely absent in CI) by
exercising :class:`PassphraseBackend` and :class:`NoneBackend` directly.
"""

from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import patch

import httpx
import pytest

from ohd_care.credentials import (
    OperatorCredentials,
    load_full_credentials,
    save_credentials,
)
from ohd_care.kms import (
    KmsBackendUnavailable,
    KmsDecryptError,
    NoneBackend,
    PassphraseBackend,
    select_backend,
)
from ohd_care.oidc import (
    DeviceCodeResponse,
    DiscoveryDocument,
    OidcDeviceFlowError,
    OidcDiscoveryError,
    discover,
    poll_device_token,
    refresh_access_token,
    start_device_flow,
)


@pytest.fixture
def isolated_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    home = tmp_path / "ohd-care"
    home.mkdir()
    monkeypatch.setenv("OHD_CARE_HOME", str(home))
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    return home


# ---------------------------------------------------------------------------
# KMS round-trip
# ---------------------------------------------------------------------------

def test_none_backend_round_trip() -> None:
    backend = NoneBackend()
    plaintext = b"storage_url = \"http://example.test\"\n"
    envelope = backend.encrypt(plaintext)
    assert envelope["kms"] == "none"
    assert "plaintext_b64" in envelope
    assert backend.decrypt(envelope) == plaintext


def test_passphrase_backend_round_trip() -> None:
    # Use weakened parameters — the production defaults take ~100ms each
    # which would slow this test.
    backend = PassphraseBackend(
        _passphrase_provider=lambda: "hunter2",
        n=2**10,
        r=8,
        p=1,
    )
    plaintext = b"sensitive vault contents"
    envelope = backend.encrypt(plaintext)
    assert envelope["kms"] == "passphrase"
    assert envelope["kms_meta"]["kdf"] == "scrypt"
    decoded = backend.decrypt(envelope)
    assert decoded == plaintext


def test_passphrase_backend_wrong_passphrase_fails() -> None:
    enc = PassphraseBackend(_passphrase_provider=lambda: "right", n=2**10)
    envelope = enc.encrypt(b"data")
    dec = PassphraseBackend(_passphrase_provider=lambda: "wrong", n=2**10)
    with pytest.raises(KmsDecryptError):
        dec.decrypt(envelope)


def test_credentials_save_load_round_trip(isolated_home: Path) -> None:
    backend = NoneBackend()
    creds = OperatorCredentials(
        storage_url="http://localhost:8443",
        access_token="ohdo_test123",
        refresh_token="ohdr_refresh",
        access_expires_at_ms=1700000000000,
        oidc_issuer="https://accounts.example.test",
        oidc_client_id="ohd-care",
        oidc_subject="abc-123",
    )
    path = save_credentials(creds, kms=backend)
    assert path.exists()
    # Envelope file is JSON now.
    raw = json.loads(path.read_text())
    assert raw["kms"] == "none"
    assert raw["version"] == 1
    # Round-trip via load.
    loaded = load_full_credentials(kms=backend)
    assert loaded.storage_url == "http://localhost:8443"
    assert loaded.access_token == "ohdo_test123"
    assert loaded.refresh_token == "ohdr_refresh"
    assert loaded.oidc_issuer == "https://accounts.example.test"
    assert loaded.oidc_subject == "abc-123"


def test_legacy_plaintext_credentials_still_load(isolated_home: Path) -> None:
    """v0.1 wrote plain TOML at credentials.toml. The loader must still parse it."""
    path = isolated_home / "credentials.toml"
    path.write_text('storage_url = "http://legacy.test"\noperator_token = "ohdo_legacy"\n')
    creds = load_full_credentials()
    assert creds.storage_url == "http://legacy.test"
    assert creds.operator_token == "ohdo_legacy"


# ---------------------------------------------------------------------------
# OIDC discovery
# ---------------------------------------------------------------------------

def _mock_transport(handler):
    return httpx.MockTransport(handler)


def test_discovery_parses_metadata(monkeypatch: pytest.MonkeyPatch) -> None:
    metadata = {
        "issuer": "https://issuer.example",
        "token_endpoint": "https://issuer.example/token",
        "device_authorization_endpoint": "https://issuer.example/device",
        "authorization_endpoint": "https://issuer.example/authorize",
    }

    def handler(request: httpx.Request) -> httpx.Response:
        assert request.url.path == "/.well-known/oauth-authorization-server"
        return httpx.Response(200, json=metadata)

    transport = _mock_transport(handler)
    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs.pop("verify", None)
        kwargs.pop("timeout", None)
        return real_client(transport=transport, **kwargs)

    monkeypatch.setattr("ohd_care.oidc.httpx.Client", fake_client)
    doc = discover("https://issuer.example")
    assert doc.token_endpoint == "https://issuer.example/token"
    assert doc.device_authorization_endpoint == "https://issuer.example/device"


def test_discovery_falls_back_to_openid_configuration(monkeypatch: pytest.MonkeyPatch) -> None:
    metadata = {
        "issuer": "https://accounts.google.test",
        "token_endpoint": "https://oauth2.google.test/token",
        "device_authorization_endpoint": "https://oauth2.google.test/device/code",
    }

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/.well-known/oauth-authorization-server":
            return httpx.Response(404)
        if request.url.path == "/.well-known/openid-configuration":
            return httpx.Response(200, json=metadata)
        return httpx.Response(500)

    transport = _mock_transport(handler)
    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs.pop("verify", None)
        kwargs.pop("timeout", None)
        return real_client(transport=transport, **kwargs)

    monkeypatch.setattr("ohd_care.oidc.httpx.Client", fake_client)
    doc = discover("https://accounts.google.test")
    assert doc.token_endpoint == "https://oauth2.google.test/token"


# ---------------------------------------------------------------------------
# Device flow polling (RFC 8628)
# ---------------------------------------------------------------------------

def _discovery() -> DiscoveryDocument:
    return DiscoveryDocument(
        issuer="https://issuer.example",
        token_endpoint="https://issuer.example/token",
        device_authorization_endpoint="https://issuer.example/device",
    )


def test_start_device_flow(monkeypatch: pytest.MonkeyPatch) -> None:
    payload = {
        "device_code": "DC1",
        "user_code": "ABCD-WXYZ",
        "verification_uri": "https://issuer.example/device",
        "expires_in": 600,
        "interval": 5,
    }

    def handler(request: httpx.Request) -> httpx.Response:
        assert request.url.path == "/device"
        body = dict(item.split("=") for item in request.content.decode().split("&"))
        assert body["client_id"] == "ohd-care"
        return httpx.Response(200, json=payload)

    transport = _mock_transport(handler)
    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs.pop("verify", None)
        kwargs.pop("timeout", None)
        return real_client(transport=transport, **kwargs)

    monkeypatch.setattr("ohd_care.oidc.httpx.Client", fake_client)
    dev = start_device_flow(_discovery(), client_id="ohd-care")
    assert dev.device_code == "DC1"
    assert dev.user_code == "ABCD-WXYZ"
    assert dev.interval == 5


def test_poll_device_token_handles_pending_then_success(monkeypatch: pytest.MonkeyPatch) -> None:
    state = {"calls": 0}

    def handler(request: httpx.Request) -> httpx.Response:
        state["calls"] += 1
        if state["calls"] == 1:
            return httpx.Response(400, json={"error": "authorization_pending"})
        if state["calls"] == 2:
            return httpx.Response(400, json={"error": "slow_down"})
        return httpx.Response(
            200,
            json={
                "access_token": "ohdo_xyz",
                "refresh_token": "ohdr_xyz",
                "token_type": "Bearer",
                "expires_in": 1800,
                "oidc_subject": "user-42",
            },
        )

    transport = _mock_transport(handler)
    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs.pop("verify", None)
        kwargs.pop("timeout", None)
        return real_client(transport=transport, **kwargs)

    monkeypatch.setattr("ohd_care.oidc.httpx.Client", fake_client)

    fake_now = [0.0]

    def now() -> float:
        return fake_now[0]

    def sleep(t: float) -> None:
        fake_now[0] += t

    device = DeviceCodeResponse(
        device_code="DC1",
        user_code="USR",
        verification_uri="https://x",
        verification_uri_complete=None,
        expires_in=600,
        interval=1,
    )
    token = poll_device_token(
        _discovery(),
        device,
        client_id="ohd-care",
        now=now,
        sleep=sleep,
    )
    assert token.access_token == "ohdo_xyz"
    assert token.refresh_token == "ohdr_xyz"
    assert token.oidc_subject == "user-42"
    assert state["calls"] == 3


def test_poll_device_token_terminal_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(400, json={"error": "access_denied", "error_description": "user denied"})

    transport = _mock_transport(handler)
    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs.pop("verify", None)
        kwargs.pop("timeout", None)
        return real_client(transport=transport, **kwargs)

    monkeypatch.setattr("ohd_care.oidc.httpx.Client", fake_client)
    device = DeviceCodeResponse(
        device_code="DC1",
        user_code="USR",
        verification_uri="https://x",
        verification_uri_complete=None,
        expires_in=600,
        interval=1,
    )
    with pytest.raises(OidcDeviceFlowError) as ei:
        poll_device_token(_discovery(), device, client_id="ohd-care", now=lambda: 0.0, sleep=lambda _: None)
    assert ei.value.code == "access_denied"


def test_refresh_access_token(monkeypatch: pytest.MonkeyPatch) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        body = dict(item.split("=") for item in request.content.decode().split("&"))
        assert body["grant_type"] == "refresh_token"
        assert body["refresh_token"] == "ohdr_old"
        return httpx.Response(
            200,
            json={
                "access_token": "ohdo_new",
                "refresh_token": "ohdr_new",
                "token_type": "Bearer",
                "expires_in": 1800,
            },
        )

    transport = _mock_transport(handler)
    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs.pop("verify", None)
        kwargs.pop("timeout", None)
        return real_client(transport=transport, **kwargs)

    monkeypatch.setattr("ohd_care.oidc.httpx.Client", fake_client)
    new = refresh_access_token(_discovery(), "ohdr_old", client_id="ohd-care")
    assert new.access_token == "ohdo_new"
    assert new.refresh_token == "ohdr_new"
