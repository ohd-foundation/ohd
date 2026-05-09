"""Helper for constructing :class:`OhdcClient` with operator-side audit context.

Per ``../spec/care-auth.md`` "Two-sided audit", every OHDC call from
Care to a patient's storage produces audit entries on both sides. The
patient-side audit row records ``actor_type='grant'`` plus
``grant_id``; the operator-side mirror needs the OIDC subject of the
clinician who actually fired the call.

We attach that subject as a header on every outgoing OHDC request.
Storage ignores it today; once storage's audit logic wires
operator-binding (see storage roadmap) it'll show up in the JOIN.
"""

from __future__ import annotations

from contextlib import contextmanager
from typing import Iterator

from .credentials import load_full_credentials
from .ohdc_client import OhdcClient


def _resolve_operator_subject() -> str | None:
    """Best-effort read of the operator's OIDC subject from the vault.

    Returns ``None`` if there's no credentials file or the subject
    isn't set (e.g. legacy `login` flow without `oidc-login`). The
    calling site stays functional in either case.
    """
    try:
        creds = load_full_credentials()
    except Exception:
        return None
    return creds.oidc_subject


@contextmanager
def build_client(storage_url: str, *, grant_token: str | None = None) -> Iterator[OhdcClient]:
    """Yield a configured :class:`OhdcClient` with the operator-subject header.

    The client doesn't open the network connection until the first
    RPC; teardown happens via the context manager.
    """
    operator_subject = _resolve_operator_subject()
    client = OhdcClient(
        storage_url=storage_url,
        bearer_token=grant_token,
        operator_subject=operator_subject,
    )
    try:
        yield client
    finally:
        client.close()
