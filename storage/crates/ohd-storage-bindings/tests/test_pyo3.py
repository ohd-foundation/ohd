"""PyO3 wheel smoke tests.

Run with:

    cd storage/crates/ohd-storage-bindings
    pip install -e ".[dev]"
    pytest tests/

Each test creates a fresh temp-dir-backed storage file so they're
independent and idempotent. The 32-byte SQLCipher key (or empty for
unencrypted) is provided as a hex string per the OhdStorage.create()
signature.
"""
from __future__ import annotations

import os
import re
import tempfile

import pytest

import ohd_storage


# Default unencrypted (`key_hex=""`) — fine for tests; production callers
# pass the per-user 32-byte hex key from their keystore.
KEY_HEX = ""


@pytest.fixture
def storage(tmp_path):
    """Open a fresh storage file under a pytest tmp_path. Re-opened per
    test so there's no shared state."""
    db_path = str(tmp_path / "ohd.db")
    return ohd_storage.OhdStorage.create(path=db_path, key_hex=KEY_HEX)


def test_module_constants():
    """Top-level version constants are present and non-empty."""
    assert ohd_storage.format_version() == "1.0"
    assert ohd_storage.protocol_version() == "ohdc.v0"
    assert ohd_storage.storage_version()  # non-empty
    assert ohd_storage.FORMAT_VERSION == "1.0"
    assert ohd_storage.PROTOCOL_VERSION == "ohdc.v0"
    assert ohd_storage.__version__  # mirrors STORAGE_VERSION


def test_open_create_roundtrip(tmp_path):
    """Storage.create on a missing file makes one; Storage.open then succeeds."""
    db_path = str(tmp_path / "ohd.db")
    s1 = ohd_storage.OhdStorage.create(path=db_path, key_hex=KEY_HEX)
    ulid = s1.user_ulid()
    assert ulid
    assert re.fullmatch(r"[0-9A-HJKMNP-TV-Z]{26}", ulid), f"not Crockford: {ulid}"
    # Drop s1, reopen.
    del s1
    s2 = ohd_storage.OhdStorage.open(path=db_path, key_hex=KEY_HEX)
    assert s2.user_ulid() == ulid
    assert s2.path() == db_path


def test_open_missing_file_raises(tmp_path):
    """Opening a non-existent file raises an OhdError subclass.

    The exact subclass is `NotFound` for missing-file opens (per
    core::Error's mapping); we match the umbrella `OhdError` so the test
    survives a future tightening of the mapping.
    """
    missing = str(tmp_path / "does-not-exist.db")
    with pytest.raises(ohd_storage.OhdError):
        ohd_storage.OhdStorage.open(path=missing, key_hex=KEY_HEX)


def test_issue_self_session_token(storage):
    """Self-session token is `ohds_<base32>`, returned cleartext exactly once."""
    token = storage.issue_self_session_token()
    assert token.startswith("ohds_"), f"unexpected prefix: {token!r}"
    # Body is non-empty (length depends on the random body in core::auth).
    assert len(token) > len("ohds_")


def test_put_event_committed(storage):
    """A well-formed put_event commits and returns a ULID."""
    ev = ohd_storage.EventInputDto(
        timestamp_ms=1_700_000_000_000,
        event_type="std.blood_glucose",
        channels=[
            ohd_storage.ChannelValueDto(
                channel_path="value",
                value_kind=ohd_storage.ValueKind.REAL,
                real_value=5.4,
            )
        ],
    )
    outcome = storage.put_event(ev)
    assert outcome.outcome == "committed", (
        f"expected committed, got {outcome.outcome!r}: {outcome.error_message}"
    )
    assert outcome.ulid
    assert re.fullmatch(r"[0-9A-HJKMNP-TV-Z]{26}", outcome.ulid)
    assert outcome.timestamp_ms > 0


def test_query_events_round_trip(storage):
    """put_event → query_events returns the row with matching ULID + value."""
    ev = ohd_storage.EventInputDto(
        timestamp_ms=1_700_000_000_000,
        event_type="std.blood_glucose",
        channels=[
            ohd_storage.ChannelValueDto(
                channel_path="value",
                value_kind=ohd_storage.ValueKind.REAL,
                real_value=5.4,
            )
        ],
    )
    outcome = storage.put_event(ev)
    assert outcome.outcome == "committed"
    written_ulid = outcome.ulid

    # Pull it back.
    flt = ohd_storage.EventFilterDto(
        from_ms=1_699_999_999_999,
        to_ms=1_700_000_000_001,
    )
    rows = storage.query_events(flt)
    assert len(rows) == 1, f"expected 1 row, got {len(rows)}: {rows!r}"
    row = rows[0]
    assert row.ulid == written_ulid
    assert row.event_type == "std.blood_glucose"
    assert row.timestamp_ms == 1_700_000_000_000
    assert len(row.channels) == 1
    cv = row.channels[0]
    assert cv.channel_path == "value"
    assert cv.value_kind == ohd_storage.ValueKind.REAL
    assert cv.real_value == pytest.approx(5.4)


def test_query_events_filter_by_event_type(storage):
    """event_types_in restricts the result set; event_types_not_in excludes."""
    base_ts = 1_700_000_000_000
    storage.put_event(
        ohd_storage.EventInputDto(
            timestamp_ms=base_ts,
            event_type="std.blood_glucose",
            channels=[ohd_storage.ChannelValueDto(
                channel_path="value",
                value_kind=ohd_storage.ValueKind.REAL,
                real_value=5.4,
            )],
        )
    )
    storage.put_event(
        ohd_storage.EventInputDto(
            timestamp_ms=base_ts + 60_000,
            event_type="std.heart_rate_resting",
            channels=[ohd_storage.ChannelValueDto(
                channel_path="value",
                value_kind=ohd_storage.ValueKind.REAL,
                real_value=64.0,
            )],
        )
    )

    rows_all = storage.query_events(ohd_storage.EventFilterDto(
        from_ms=base_ts - 1, to_ms=base_ts + 60_001,
    ))
    assert len(rows_all) == 2

    rows_glucose = storage.query_events(ohd_storage.EventFilterDto(
        from_ms=base_ts - 1, to_ms=base_ts + 60_001,
        event_types_in=["std.blood_glucose"],
    ))
    assert len(rows_glucose) == 1
    assert rows_glucose[0].event_type == "std.blood_glucose"

    rows_no_glucose = storage.query_events(ohd_storage.EventFilterDto(
        from_ms=base_ts - 1, to_ms=base_ts + 60_001,
        event_types_not_in=["std.blood_glucose"],
    ))
    assert len(rows_no_glucose) == 1
    assert rows_no_glucose[0].event_type == "std.heart_rate_resting"


def test_unknown_event_type_returns_error_outcome(storage):
    """An unknown event_type comes back as outcome='error' with a code.

    Per `put_events` semantics, per-event validation failures don't raise —
    they surface as one `PutEventOutcomeDto(outcome='error', error_code=...)`
    so callers can keep going after a partial-batch reject. (Cross-call
    failures like missing storage file *do* raise.)
    """
    ev = ohd_storage.EventInputDto(
        timestamp_ms=1_700_000_000_000,
        event_type="nonexistent.bogus_type",
        channels=[],
    )
    outcome = storage.put_event(ev)
    assert outcome.outcome == "error", outcome
    assert outcome.error_code, outcome
    assert outcome.error_message, outcome


def test_value_kind_mismatch_raises_invalid_input(storage):
    """REAL value_kind without real_value rejects with InvalidInput."""
    ev = ohd_storage.EventInputDto(
        timestamp_ms=1_700_000_000_000,
        event_type="std.blood_glucose",
        channels=[
            ohd_storage.ChannelValueDto(
                channel_path="value",
                value_kind=ohd_storage.ValueKind.REAL,
                # no real_value set
            )
        ],
    )
    with pytest.raises(ohd_storage.InvalidInput):
        storage.put_event(ev)


def test_exception_hierarchy(storage):
    """All five concrete exceptions subclass OhdError, which subclasses RuntimeError."""
    assert issubclass(ohd_storage.OpenFailed, ohd_storage.OhdError)
    assert issubclass(ohd_storage.Auth, ohd_storage.OhdError)
    assert issubclass(ohd_storage.InvalidInput, ohd_storage.OhdError)
    assert issubclass(ohd_storage.NotFound, ohd_storage.OhdError)
    assert issubclass(ohd_storage.Internal, ohd_storage.OhdError)
    assert issubclass(ohd_storage.OhdError, RuntimeError)

    # Catching with the root class works (concrete subclass is NotFound for
    # the missing-path case, but OhdError catches it).
    with pytest.raises(ohd_storage.OhdError):
        ohd_storage.OhdStorage.open(path="/no/such/path/data.db", key_hex="")


def test_value_kind_enum():
    """ValueKind is comparable + reprs sensibly.

    Note: pyclass-with-`eq` enums aren't hashable in PyO3 (no `Hash`
    derived); compare pairwise for distinctness instead of via a set.
    """
    assert ohd_storage.ValueKind.REAL == ohd_storage.ValueKind.REAL
    assert ohd_storage.ValueKind.REAL != ohd_storage.ValueKind.INT
    variants = [
        ohd_storage.ValueKind.REAL,
        ohd_storage.ValueKind.INT,
        ohd_storage.ValueKind.BOOL,
        ohd_storage.ValueKind.TEXT,
        ohd_storage.ValueKind.ENUM_ORDINAL,
    ]
    # Pairwise distinct.
    for i, a in enumerate(variants):
        for j, b in enumerate(variants):
            if i == j:
                assert a == b
            else:
                assert a != b
    # __repr__ pins the variant name.
    assert "REAL" in repr(ohd_storage.ValueKind.REAL)
    assert "ENUM_ORDINAL" in repr(ohd_storage.ValueKind.ENUM_ORDINAL)


def test_format_and_protocol_methods_on_handle(storage):
    """Storage handle exposes format_version() and protocol_version() too."""
    assert storage.format_version() == "1.0"
    assert storage.protocol_version() == "ohdc.v0"


# ---------------------------------------------------------------------------
# Grants — list_grants / create_grant / revoke_grant.
# ---------------------------------------------------------------------------

def test_list_grants_empty_initially(storage):
    """A fresh storage has no grants."""
    out = storage.list_grants(ohd_storage.ListGrantsFilterDto())
    assert out == []


def test_create_grant_then_list_then_revoke(storage):
    """Round-trip: create_grant returns a (ulid, token, share_url) triple;
    list_grants picks it up; revoke_grant flips revoked_at_ms."""
    cg = ohd_storage.CreateGrantInputDto(
        grantee_label="Dr E2E",
        grantee_kind="human",
        default_action="deny",
        approval_mode="always",
        event_type_rules=[
            ohd_storage.GrantEventTypeRuleDto("std.blood_glucose", "allow"),
        ],
    )
    out = storage.create_grant(cg)
    assert out.grant_ulid
    assert out.token.startswith("ohdg_")
    assert out.share_url.startswith("ohd://grant/")

    grants = storage.list_grants(ohd_storage.ListGrantsFilterDto())
    assert len(grants) == 1
    assert grants[0].grantee_label == "Dr E2E"
    assert grants[0].grantee_kind == "human"
    assert grants[0].revoked_at_ms is None

    storage.revoke_grant(out.grant_ulid)
    grants_after = storage.list_grants(
        ohd_storage.ListGrantsFilterDto(include_revoked=True)
    )
    assert len(grants_after) == 1
    assert grants_after[0].revoked_at_ms is not None


# ---------------------------------------------------------------------------
# Emergency config — get / set round-trip.
# ---------------------------------------------------------------------------

def test_emergency_config_defaults(storage):
    """Fresh storage returns the spec's default emergency config."""
    cfg = storage.get_emergency_config()
    assert cfg.enabled is False
    assert cfg.approval_timeout_seconds == 30
    assert cfg.default_action_on_timeout == "allow"
    assert cfg.history_window_hours == 24
    assert cfg.bystander_proxy_enabled is True


def test_emergency_config_set_round_trip(storage):
    """Set then get returns what was written; updated_at_ms is bumped."""
    new = ohd_storage.EmergencyConfigDto(
        enabled=True,
        approval_timeout_seconds=45,
        default_action_on_timeout="refuse",
        lock_screen_visibility="basic_only",
        history_window_hours=12,
        share_location=True,
        bystander_proxy_enabled=False,
    )
    storage.set_emergency_config(new)
    got = storage.get_emergency_config()
    assert got.enabled is True
    assert got.approval_timeout_seconds == 45
    assert got.default_action_on_timeout == "refuse"
    assert got.lock_screen_visibility == "basic_only"
    assert got.history_window_hours == 12
    assert got.share_location is True
    assert got.bystander_proxy_enabled is False
    assert got.updated_at_ms > 0


def test_emergency_config_invalid_timeout_raises(storage):
    """Out-of-range timeout (>300s) is rejected with InvalidInput."""
    bad = ohd_storage.EmergencyConfigDto(
        enabled=True,
        approval_timeout_seconds=999,
    )
    with pytest.raises(ohd_storage.InvalidInput):
        storage.set_emergency_config(bad)


# ---------------------------------------------------------------------------
# Source signing — register / list / revoke.
# ---------------------------------------------------------------------------

# Sample Ed25519 SubjectPublicKeyInfo PEM. Generated once with
# `openssl genpkey -algorithm Ed25519` then `openssl pkey -pubout`. Any
# valid Ed25519 SPKI works — the tests don't actually verify with this key.
ED25519_PEM = """-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=
-----END PUBLIC KEY-----
"""


def test_signer_registry_round_trip(storage):
    """register_signer → list_signers → revoke_signer."""
    sig = storage.register_signer(
        "libre.eu.2026-01",
        "Libre EU production",
        "ed25519",
        ED25519_PEM,
    )
    assert sig.signer_kid == "libre.eu.2026-01"
    assert sig.sig_alg == "ed25519"
    assert sig.revoked_at_ms is None

    listed = storage.list_signers()
    assert len(listed) == 1
    assert listed[0].signer_kid == "libre.eu.2026-01"

    storage.revoke_signer("libre.eu.2026-01")
    listed_after = storage.list_signers()
    # revoke_signer doesn't delete; it sets revoked_at_ms.
    assert len(listed_after) == 1
    assert listed_after[0].revoked_at_ms is not None


# ---------------------------------------------------------------------------
# Export — bytes round-trip.
# ---------------------------------------------------------------------------

def test_export_all_returns_bytes(storage):
    """export_all on an empty storage returns a non-empty CBOR blob (the
    init/finish frames alone)."""
    blob = storage.export_all()
    assert isinstance(blob, (bytes, bytearray))
    assert len(blob) > 0
