"""Re-export shim: canonical_query_hash now lives in ``ohd_shared``.

This module previously held a byte-identical copy of the canonical query-hash
algorithm. It now re-exports the single source from ``ohd_shared`` so the
existing call sites (``ohd_care.ohdc_client``, the CLI tests, anything that
``import`` s ``ohd_care.canonical_query_hash`` directly) keep working
unchanged.
"""

from __future__ import annotations

from ohd_shared.canonical_query_hash import (
    CanonicalChannelPredicate,
    CanonicalEventFilter,
    CanonicalQueryKind,
    canonical_filter_json,
    canonical_query_hash,
)

__all__ = [
    "CanonicalChannelPredicate",
    "CanonicalEventFilter",
    "CanonicalQueryKind",
    "canonical_filter_json",
    "canonical_query_hash",
]
