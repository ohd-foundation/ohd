"""OHD Care CLI — terminal interface for the OHD reference clinical app.

The CLI talks **OHDC over Connect-RPC** to a running ``ohd-storage-server``
under a per-patient grant token. The active patient is set with
``ohd-care use <label>`` and stored in ``~/.config/ohd-care/active.toml``;
each grant token lives in ``~/.config/ohd-care/grants/<label>.toml``
(mode 0600).

See ``../SPEC.md`` for the operator-side contract and ``./STATUS.md`` for
the current wiring state.
"""

from __future__ import annotations

__version__ = "0.1.0"

__all__ = ["__version__"]
