#!/usr/bin/env bash
# Regenerate the OHDC v0 protobuf Python stubs into
# ``src/ohd_shared/_gen/ohdc/v0/``.
#
# This is the single source of truth for the proto stubs; the per-MCP
# ``_gen/`` directories were removed when the shared package landed
# (`packaging/python/ohd-shared/`). Run from any directory; the script
# resolves paths relative to the repo layout. Requires the
# ``grpcio-tools`` dev dep (pinned in this package's ``pyproject.toml``).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMP_DIR="$(cd "$HERE/.." && pwd)"
PROTO_ROOT="$(cd "$COMP_DIR/../../../storage/proto" && pwd)"
OUT_DIR="$COMP_DIR/src/ohd_shared/_gen"

mkdir -p "$OUT_DIR/ohdc/v0"
touch "$OUT_DIR/__init__.py" "$OUT_DIR/ohdc/__init__.py" "$OUT_DIR/ohdc/v0/__init__.py"

cd "$COMP_DIR"
uv run python -m grpc_tools.protoc \
  --proto_path="$PROTO_ROOT" \
  --python_out="$OUT_DIR" \
  ohdc/v0/ohdc.proto

echo "regenerated: $OUT_DIR/ohdc/v0/ohdc_pb2.py"
