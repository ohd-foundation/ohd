#!/usr/bin/env bash
# OHD Connect — end-to-end demo / cross-component integration test.
#
# Run with `bash connect/demo/run.sh` (the file may not have +x in a fresh
# checkout — the harness that scaffolded it doesn't always chmod).
#
# Purpose: prove that `ohd-connect` (./cli/) talks to a real `ohd-storage-server`
# (../../storage/) over Connect-RPC and round-trips an event through PutEvents
# → QueryEvents.
#
# Steps:
#   1. Build both binaries (storage server + CLI). Reuses existing target/
#      caches; first cold run does the full SQLCipher C compile (~3 min).
#   2. Start the storage server on a private temp DB at an unprivileged port.
#   3. Issue a self-session token via `ohd-storage-server issue-self-token`.
#   4. Drive the CLI:
#        ohd-connect login → whoami → log glucose 6.4 → query glucose --last-day
#      and assert the round-trip prints the event back.
#   5. Tear down the storage server and clean temp state.
#
# Constraints:
#   - Plaintext h2c only — TLS is the deployment's job (Caddy fronts the
#     storage process per ../../storage/STATUS.md "HTTP/3 deferred").
#   - Uses an unprivileged TCP port (default 18443) so a non-root developer
#     can run this without sudo. Override with $OHD_PORT.
#   - Honours an isolated credentials dir (XDG_CONFIG_HOME) so the demo
#     doesn't clobber a developer's real ~/.config/ohd-connect/.
#
# Exit non-zero on any failure (set -e); the "expected" assertions are visible
# greps so a CI log is enough to debug.

set -euo pipefail

# -- Layout ------------------------------------------------------------------
DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECT_DIR="$(cd "$DEMO_DIR/.." && pwd)"
OHD_ROOT="$(cd "$CONNECT_DIR/.." && pwd)"
STORAGE_DIR="$OHD_ROOT/storage"
CLI_DIR="$CONNECT_DIR/cli"

PORT="${OHD_PORT:-18443}"
ADDR="127.0.0.1:${PORT}"
STORAGE_URL="http://${ADDR}"

WORKDIR="$(mktemp -d -t ohd-connect-demo.XXXXXX)"
DB="$WORKDIR/data.db"
LOG="$WORKDIR/storage.log"
export XDG_CONFIG_HOME="$WORKDIR/xdg-config"
mkdir -p "$XDG_CONFIG_HOME"

# -- Cleanup ----------------------------------------------------------------
SERVER_PID=""
cleanup() {
    if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    if [[ -n "${KEEP_WORKDIR:-}" ]]; then
        echo "[demo] keeping workdir for inspection: $WORKDIR"
    else
        rm -rf "$WORKDIR"
    fi
}
trap cleanup EXIT

# -- Build ------------------------------------------------------------------
echo "[demo] building ohd-storage-server (release of work, debug for speed)..."
( cd "$STORAGE_DIR" && cargo build -q -p ohd-storage-server )
echo "[demo] building ohd-connect..."
( cd "$CLI_DIR" && cargo build -q )

STORAGE_BIN="$STORAGE_DIR/target/debug/ohd-storage-server"
CLI_BIN="$CLI_DIR/target/debug/ohd-connect"
[[ -x "$STORAGE_BIN" ]] || { echo "[demo] missing $STORAGE_BIN" >&2; exit 1; }
[[ -x "$CLI_BIN"     ]] || { echo "[demo] missing $CLI_BIN"     >&2; exit 1; }

# -- Initialize storage file ------------------------------------------------
echo "[demo] initialising storage at $DB"
"$STORAGE_BIN" init --db "$DB"

# -- Mint a self-session token ---------------------------------------------
echo "[demo] minting self-session token..."
TOKEN="$("$STORAGE_BIN" issue-self-token --db "$DB" --label "demo")"
[[ "$TOKEN" =~ ^ohds_ ]] || { echo "[demo] expected ohds_ prefix, got: $TOKEN" >&2; exit 1; }
echo "[demo] token: ${TOKEN:0:12}…"

# -- Start the storage server ----------------------------------------------
echo "[demo] starting ohd-storage-server on $ADDR (log: $LOG)"
"$STORAGE_BIN" serve --db "$DB" --listen "$ADDR" >"$LOG" 2>&1 &
SERVER_PID=$!

# Wait for the listener to come up. The server logs "OHDC Connect-RPC
# listening" via tracing; poll the TCP port instead so we don't depend on
# the log format.
for _ in $(seq 1 50); do
    if (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null; then
        break
    fi
    sleep 0.1
done
if ! (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null; then
    echo "[demo] server failed to bind $ADDR; log:" >&2
    cat "$LOG" >&2 || true
    exit 1
fi

# -- CLI: login ------------------------------------------------------------
echo "[demo] === ohd-connect login ==="
"$CLI_BIN" login --storage "$STORAGE_URL" --token "$TOKEN"

# -- CLI: whoami -----------------------------------------------------------
echo "[demo] === ohd-connect whoami ==="
WHOAMI_OUT="$("$CLI_BIN" whoami)"
echo "$WHOAMI_OUT"
echo "$WHOAMI_OUT" | grep -q "^token_kind: *self_session$" \
    || { echo "[demo] whoami did not show token_kind=self_session" >&2; exit 1; }

# -- CLI: health -----------------------------------------------------------
echo "[demo] === ohd-connect health ==="
HEALTH_OUT="$("$CLI_BIN" health)"
echo "$HEALTH_OUT"
echo "$HEALTH_OUT" | grep -q "^status: *ok$" \
    || { echo "[demo] health did not return status=ok" >&2; exit 1; }

# -- CLI: log glucose ------------------------------------------------------
echo "[demo] === ohd-connect log glucose 6.4 ==="
LOG_OUT="$("$CLI_BIN" log glucose 6.4)"
echo "$LOG_OUT"
echo "$LOG_OUT" | grep -q "^committed " \
    || { echo "[demo] log glucose did not commit" >&2; exit 1; }

# -- CLI: log glucose with mg/dL conversion --------------------------------
echo "[demo] === ohd-connect log glucose 120 --unit mg/dL ==="
"$CLI_BIN" log glucose 120 --unit mg/dL
# (committed → ulid; just smoke test that the command exits 0)

# -- CLI: query glucose ----------------------------------------------------
echo "[demo] === ohd-connect query glucose --last-day ==="
QUERY_OUT="$("$CLI_BIN" query glucose --last-day 2>&1)"
echo "$QUERY_OUT"
# Expect at least the column header + 2 rows + the "(N events)" footer.
echo "$QUERY_OUT" | grep -q "^ULID " \
    || { echo "[demo] query did not print header" >&2; exit 1; }
echo "$QUERY_OUT" | grep -q "value=" \
    || { echo "[demo] query did not print a value channel" >&2; exit 1; }
echo "$QUERY_OUT" | grep -qE "\\(2 events\\)" \
    || { echo "[demo] expected 2 events from the round-trip" >&2; exit 1; }

# -- Done ------------------------------------------------------------------
echo
echo "[demo] OK — round-tripped through OHD Storage via Connect-RPC."
