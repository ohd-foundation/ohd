#!/usr/bin/env bash
# Regenerate the Kotlin uniffi bindings for the Android app.
#
# Two non-obvious requirements `--library` mode has that the README's command
# silently mishandles:
#   1. Use the **debug** cdylib — `cargo build -p ohd-storage-bindings`. The
#      release profile strips the uniffi metadata symbols `--library` mode
#      reads, so a release-built .so silently produces zero output (exit 0,
#      no file written).
#   2. `--out-dir` is the Kotlin **source root** (`.../java`), not the
#      `uniffi/` subdir. The bindgen appends `uniffi/<namespace>/` itself.
#
# This script handles both, then applies the Kotlin-2.0 `override val message`
# patch (the generator emits a constructor `val message` and a separate
# `override val message get()`, which Kotlin 2.0 rejects as conflicting).

set -euo pipefail

# Locate the workspace root from this script's directory.
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
STORAGE="$ROOT/storage"
SO="$STORAGE/target/debug/libohd_storage_bindings.so"
OUT="$ROOT/connect/android/app/src/main/java"
KT="$OUT/uniffi/ohd_storage/ohd_storage.kt"

echo "==> building debug cdylib"
(cd "$STORAGE" && cargo build -p ohd-storage-bindings)

echo "==> generating Kotlin"
(cd "$STORAGE" && cargo run -q -p ohd-storage-bindings --features cli \
    --bin uniffi-bindgen -- generate \
    --library "$SO" \
    --language kotlin \
    --out-dir "$OUT")

echo "==> applying Kotlin-2.0 override val message patch"
python3 - "$KT" <<'PY'
import re, sys
path = sys.argv[1]
with open(path) as f: s = f.read()
# Pattern 1: lone `val \`message\``
p1 = re.compile(
    r"        val `message`: kotlin\.String\n"
    r"        \) : OhdException\(\) \{\n"
    r"        override val message\n"
    r'            get\(\) = "message=\$\{ `message` \}"\n'
    r"    \}"
)
# Pattern 2: `code` + `message` form (most variants).
p2 = re.compile(
    r"        val `message`: kotlin\.String\n"
    r"        \) : OhdException\(\) \{\n"
    r"        override val message\n"
    r'            get\(\) = "code=\$\{ `code` \}, message=\$\{ `message` \}"\n'
    r"    \}"
)
repl = (
    "        override val `message`: kotlin.String\n"
    "        ) : OhdException() {\n"
    "    }"
)
n1 = len(p1.findall(s)); s = p1.sub(repl, s)
n2 = len(p2.findall(s)); s = p2.sub(repl, s)
with open(path, 'w') as f: f.write(s)
print(f"  patched {n1} single-message + {n2} code+message variants")
PY

echo "==> done — $KT"
