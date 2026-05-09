#!/usr/bin/env bash
# OHD end-to-end demo — drives the 11 steps in care/STATUS.md "Demo target".
#
# What it does:
#   1. Build the storage server + the connect CLI (debug/release bins) if missing.
#   2. Init a fresh per-user file at /tmp/ohd-demo.db.
#   3. Issue a self-session token + a grant token (Dr. Smith, approval-mode=always,
#      read=glucose+hr+temp+meds+symptom, write=clinical_note).
#   4. Log 5 mock events via `ohd-connect log` so the patient view has data.
#   5. Boot the storage server in the background (CORS permissive on port 18443).
#   6. Print the URL the user should open in their browser.
#   7. Wait for ENTER, then keep printing helper commands as the user clicks
#      around the UI (submit a clinical note → pending list → pending approve).
#
# This script is idempotent within reason — re-running re-uses the same token
# files but tears down the server first. The DB file is wiped on each run.
#
# Run from anywhere; we resolve all paths relative to the repo root.

set -Eeuo pipefail
shopt -s inherit_errexit

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
STORAGE_DIR="$REPO_ROOT/storage"
CONNECT_DIR="$REPO_ROOT/connect"
CARE_WEB_DIR="$REPO_ROOT/care/web"
DEMO_DIR="$REPO_ROOT/care/demo"

DB_PATH="${OHD_DEMO_DB:-/tmp/ohd-demo.db}"
LISTEN_ADDR="${OHD_DEMO_LISTEN:-0.0.0.0:18443}"
PUBLIC_URL="${OHD_DEMO_URL:-http://localhost:18443}"
WEB_URL="${OHD_DEMO_WEB_URL:-http://localhost:5173}"

OHDS_TOKEN_FILE="$DEMO_DIR/.last-self-token"
OHDG_TOKEN_FILE="$DEMO_DIR/.last-grant-token"
SERVER_PID_FILE="$DEMO_DIR/.server.pid"
SERVER_LOG_FILE="$DEMO_DIR/.server.log"

STORAGE_BIN="$STORAGE_DIR/target/debug/ohd-storage-server"
CONNECT_BIN="$CONNECT_DIR/cli/target/release/ohd-connect"

bold()    { printf '\033[1m%s\033[0m\n' "$*"; }
section() { printf '\n\033[1;36m== %s ==\033[0m\n' "$*"; }
note()    { printf '  \033[2m· %s\033[0m\n' "$*"; }
warn()    { printf '\033[1;33m!! %s\033[0m\n' "$*"; }

cleanup() {
    if [[ -f "$SERVER_PID_FILE" ]]; then
        local pid
        pid="$(cat "$SERVER_PID_FILE" 2>/dev/null || echo)"
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            note "shutting down storage server (pid $pid)"
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
        rm -f "$SERVER_PID_FILE"
    fi
}
trap cleanup EXIT

# ---- 0. Build prerequisites ----------------------------------------------
section "Build / locate binaries"
if [[ ! -x "$STORAGE_BIN" ]]; then
    note "building ohd-storage-server (debug)"
    (cd "$STORAGE_DIR" && cargo build -p ohd-storage-server)
fi
if [[ ! -x "$CONNECT_BIN" ]]; then
    note "building ohd-connect (release)"
    (cd "$CONNECT_DIR/cli" && cargo build --release)
fi
note "storage:  $STORAGE_BIN"
note "connect:  $CONNECT_BIN"

# ---- 1. Fresh DB ---------------------------------------------------------
section "Fresh DB at $DB_PATH"
rm -f "$DB_PATH"
"$STORAGE_BIN" init --db "$DB_PATH"

# ---- 2. Self-session token (used by `ohd-connect log`) -------------------
section "Issue self-session token (for ohd-connect log)"
SELF_TOKEN="$("$STORAGE_BIN" issue-self-token --db "$DB_PATH" --label "demo-self")"
echo "$SELF_TOKEN" >"$OHDS_TOKEN_FILE"
note "ohds_… → $OHDS_TOKEN_FILE"

# ---- 3. Mock events via ohd-connect --------------------------------------
# We drive the events in-process before we boot the network server so the CLI
# talks to the same DB file (the rusqlite WAL handles single-writer + readers).
# That's not how the CLI is normally used (it speaks Connect-RPC) but it lets
# us seed the DB without spinning up the server twice.
#
# We boot the server, log the 5 events via the CLI, then keep the server up
# for the browser.

section "Boot storage server (background)"
rm -f "$SERVER_LOG_FILE"
RUST_LOG="${RUST_LOG:-info}" "$STORAGE_BIN" serve --db "$DB_PATH" --listen "$LISTEN_ADDR" \
    >"$SERVER_LOG_FILE" 2>&1 &
echo $! >"$SERVER_PID_FILE"
note "pid $(cat "$SERVER_PID_FILE") logging to $SERVER_LOG_FILE"
# Give it a beat to bind the socket.
sleep 1
if ! kill -0 "$(cat "$SERVER_PID_FILE")" 2>/dev/null; then
    warn "storage server died on boot — see $SERVER_LOG_FILE"
    tail -20 "$SERVER_LOG_FILE" || true
    exit 1
fi

section "Seed 5 events via ohd-connect"
export OHD_CONNECT_STORAGE="$PUBLIC_URL"
"$CONNECT_BIN" --storage "$PUBLIC_URL" --token "$SELF_TOKEN" log glucose 120
"$CONNECT_BIN" --storage "$PUBLIC_URL" --token "$SELF_TOKEN" log glucose 138
"$CONNECT_BIN" --storage "$PUBLIC_URL" --token "$SELF_TOKEN" log heart-rate 72
"$CONNECT_BIN" --storage "$PUBLIC_URL" --token "$SELF_TOKEN" log temperature 36.7
"$CONNECT_BIN" --storage "$PUBLIC_URL" --token "$SELF_TOKEN" log symptom \
    "mild headache" --severity 3
note "5 events logged"

# ---- 4. Grant token for the doctor --------------------------------------
section "Issue grant token (Dr. Smith — approval-mode=always)"
GRANT_TOKEN="$("$STORAGE_BIN" issue-grant-token --db "$DB_PATH" \
    --read   "std.blood_glucose,std.heart_rate_resting,std.body_temperature,std.medication_dose,std.symptom" \
    --write  "std.clinical_note" \
    --approval-mode "always" \
    --label  "Dr. Smith" \
    --expires-days 30)"
echo "$GRANT_TOKEN" >"$OHDG_TOKEN_FILE"
note "ohdg_… → $OHDG_TOKEN_FILE"

# ---- 5. Print the browser URL --------------------------------------------
section "Open the Care web app"
DEMO_URL="$WEB_URL/?token=$GRANT_TOKEN"
echo
bold "  $DEMO_URL"
echo
note "Make sure the dev server is up:    cd $CARE_WEB_DIR && pnpm dev"
note "Then paste the URL above into your browser."
echo

cat <<EOF

──────────────────────────────────────────────────────────────────────────────
What you should see in the browser:
  · Roster: 1 patient card (your own user, identified by ULID prefix).
  · Patient view:
      - Vitals tab → 2 glucose, 1 HR, 1 temp readings.
      - Symptoms tab → 1 'mild headache' row.
      - Notes tab → empty (no clinical_note events yet).
  · "+ New note" on the Notes tab → submit text → toast says
    "submitted — awaiting patient approval". (The note doesn't appear yet
    because the grant's approval_mode is 'always' so the write queues into
    pending_events.)
──────────────────────────────────────────────────────────────────────────────

EOF

bold "After clicking submit in the browser, run:"
echo
echo "  $STORAGE_BIN pending-list --db $DB_PATH"
echo
bold "Then approve the pending note with its ULID:"
echo
echo "  $STORAGE_BIN pending-approve --db $DB_PATH --ulid <ULID>"
echo
bold "Reload the browser — the note now shows on the Notes tab."
echo

# ---- 6. Wait for the user --------------------------------------------------
section "Press Ctrl-C to shut down the storage server when done"
note "tail of $SERVER_LOG_FILE follows:"
echo
tail -f "$SERVER_LOG_FILE"
