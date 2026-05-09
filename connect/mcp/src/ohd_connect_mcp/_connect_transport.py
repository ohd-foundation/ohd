"""Re-export shim: Connect-RPC transport now lives in ``ohd_shared``.

This module previously held a byte-identical copy of the Connect-RPC
transport. It now re-exports the single source from
``ohd_shared.connect_transport`` so any existing imports keep working.
"""

from __future__ import annotations

from ohd_shared.connect_transport import OhdcRpcError, OhdcTransport

__all__ = [
    "OhdcRpcError",
    "OhdcTransport",
]
