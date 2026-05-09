"""Generate Python protobuf stubs for the OHDC schema.

Reads ``../../storage/proto/ohdc/v0/*.proto`` and writes Python message
classes into ``src/ohd_care/ohdc_proto/``. We only need the message types
(no service stubs) — the OHDC client in ``ohdc_client.py`` builds the
Connect-RPC wire frames by hand.

Uses ``grpcio_tools.protoc`` rather than a system ``protoc`` so the CLI's
``uv sync`` pulls everything it needs in one step.

Run manually with::

    uv run python scripts/gen_proto.py

Or it's invoked lazily on first import of ``ohd_care.ohdc_proto`` if the
generated package is missing.
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

CLI_ROOT = Path(__file__).resolve().parent.parent
PROTO_ROOT = (CLI_ROOT / ".." / ".." / "storage" / "proto").resolve()
OUT_DIR = CLI_ROOT / "src" / "ohd_care" / "ohdc_proto"


# Body of `src/ohd_care/ohdc_proto/__init__.py`. Includes a lazy codegen
# trigger so a freshly cloned tree imports without a manual codegen step,
# plus the `sys.path` insert that lets generated `from ohdc.v0 import ...`
# absolute imports resolve from inside the generated tree.
_PROTO_INIT_PY = '''\
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
    print("ohd-care: generating OHDC protobuf stubs (one-time)\\u2026", file=sys.stderr)
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
'''


def find_proto_files() -> list[Path]:
    if not PROTO_ROOT.exists():
        raise SystemExit(
            f"proto root not found: {PROTO_ROOT}\n"
            "ohd-care expects to live next to the storage tree at "
            "../../storage/proto/."
        )
    files = sorted(PROTO_ROOT.glob("ohdc/v0/*.proto"))
    if not files:
        raise SystemExit(f"no .proto files under {PROTO_ROOT}/ohdc/v0")
    return files


def ensure_out_dir() -> None:
    if OUT_DIR.exists():
        shutil.rmtree(OUT_DIR)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    # Mark as a real Python package; we add a tiny shim so users can do
    # `from ohd_care.ohdc_proto import ohdc_pb2`. The shim also performs
    # lazy codegen on first import (so a fresh `uv sync && uv run ohd-care`
    # works without a separate codegen step) and inserts this directory on
    # `sys.path` so the generated absolute imports (`from ohdc.v0 import ...`)
    # resolve.
    init_py = OUT_DIR / "__init__.py"
    init_py.write_text(_PROTO_INIT_PY)
    # Sub-package init files. `protoc --python_out` writes
    # `ohdc/v0/*_pb2.py` directly under our out dir; we need __init__.py
    # at each level so Python recognises them as packages.
    (OUT_DIR / "ohdc").mkdir(exist_ok=True)
    (OUT_DIR / "ohdc" / "__init__.py").write_text("")
    (OUT_DIR / "ohdc" / "v0").mkdir(exist_ok=True)
    (OUT_DIR / "ohdc" / "v0" / "__init__.py").write_text("")


def run_protoc(proto_files: list[Path]) -> None:
    try:
        from grpc_tools import protoc  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover - dev dep
        raise SystemExit(
            "grpc_tools.protoc is not available. Install dev deps with "
            "`uv sync --extra dev` (or `pip install grpcio-tools`)."
        ) from exc

    # `grpcio_tools` ships protoc + the standard google/protobuf well-known
    # types under its own data dir; expose that path so the imports
    # `import google.protobuf.timestamp_pb2` resolve when we re-run protoc
    # against the schema (which uses Timestamp + Duration + Struct).
    import grpc_tools

    grpc_tools_data = Path(grpc_tools.__file__).parent / "_proto"

    args = [
        "protoc",
        f"-I{PROTO_ROOT}",
        f"-I{grpc_tools_data}",
        f"--python_out={OUT_DIR}",
        f"--pyi_out={OUT_DIR}",
        *[str(p) for p in proto_files],
    ]
    rc = protoc.main(args)
    if rc != 0:
        raise SystemExit(f"protoc exited non-zero: {rc}")


def fixup_imports() -> None:
    """Rewrite generated `import ohdc.v0.foo_pb2` → relative-friendly form.

    `protoc --python_out` writes absolute imports rooted at the proto
    package. Since our generated tree lives under
    `ohd_care.ohdc_proto.ohdc.v0.*`, those bare `from ohdc.v0 import …`
    statements would only resolve if we put `src/ohd_care/ohdc_proto` on
    `sys.path` — which the lazy loader does, but it's brittle. Easier to
    leave the generated files alone and rely on the loader's path tweak.
    """
    # Currently a no-op; left as a hook in case we want to switch to
    # relative imports later.
    return


def main() -> int:
    proto_files = find_proto_files()
    ensure_out_dir()
    print(f"generating Python stubs from {len(proto_files)} proto file(s)…")
    run_protoc(proto_files)
    fixup_imports()
    print(f"wrote stubs to {OUT_DIR.relative_to(CLI_ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
