"""Generated protobuf stubs for OHDC v0.

This package is **generated**: run ``uv run python scripts/gen_proto.py``
to (re)build it from ``../../storage/proto/ohdc/v0/*.proto``.

For convenience we lazily run the codegen on first import if the package is
empty, so a freshly-cloned ``uv sync && uv run ohd-care --help`` works
without a separate codegen step.

The generated layout is::

    ohd_care.ohdc_proto.ohdc.v0.ohdc_pb2  -> messages
    ohd_care.ohdc_proto.ohdc.v0.auth_pb2
    ohd_care.ohdc_proto.ohdc.v0.relay_pb2
    ohd_care.ohdc_proto.ohdc.v0.sync_pb2
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_GEN_SCRIPT = _HERE.parent.parent.parent / "scripts" / "gen_proto.py"


def _looks_generated() -> bool:
    pb_file = _HERE / "ohdc" / "v0" / "ohdc_pb2.py"
    return pb_file.is_file()


def _ensure_generated() -> None:
    if _looks_generated():
        return
    if not _GEN_SCRIPT.is_file():
        raise ImportError(
            "OHDC protobuf stubs are missing and the codegen script is not "
            "shipped with this install. Reinstall from source with "
            "`uv sync` from the care/cli source tree."
        )
    print("ohd-care: generating OHDC protobuf stubs (one-time)\u2026", file=sys.stderr)
    res = subprocess.run(
        [sys.executable, str(_GEN_SCRIPT)],
        check=False,
        env={**os.environ, "PYTHONIOENCODING": "utf-8"},
    )
    if res.returncode != 0:
        raise ImportError(
            f"OHDC stub generation failed (rc={res.returncode}). Run "
            f"`{sys.executable} {_GEN_SCRIPT}` manually to see the error."
        )


_ensure_generated()

# Generated files use absolute imports rooted at `ohdc.v0`; expose this
# directory on sys.path so those resolve.
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))
