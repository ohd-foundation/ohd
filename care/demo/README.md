# OHD Care — End-to-End Demo

> Drives the 11-step write-with-approval flow that proves the OHD protocol's core feature: a doctor submits a clinical note via Care, the patient approves, and the note commits to the patient's file.

## What this demo does

| Step | Where | Action |
|---|---|---|
| 1 | terminal | `ohd-storage-server init --db /tmp/ohd-demo.db` |
| 2 | terminal | `issue-self-token` → `ohds_…` |
| 3 | terminal | `ohd-connect log glucose 120` × 5 |
| 4 | terminal | `issue-grant-token --read … --write std.clinical_note --approval-mode always` → `ohdg_…` |
| 5 | terminal | `ohd-storage-server serve` (port 18443, CORS permissive) |
| 6 | browser  | open `http://localhost:5173/?token=ohdg_…` |
| 7 | browser  | Roster shows 1 patient, Patient view shows real glucose / HR / temp |
| 8 | browser  | Notes tab → "+ New note" → submit `std.clinical_note` → toast "submitted, awaiting patient approval" |
| 9 | terminal | `ohd-storage-server pending-list` shows the queued note |
| 10 | terminal | `ohd-storage-server pending-approve --ulid <…>` |
| 11 | browser  | Reload → the clinical note appears on the Notes tab |

Steps 1–5 + 9 + 10 are CLI; steps 6–8 + 11 are browser interactions. The `run.sh` script automates the CLI side.

## Prerequisites

- Built `ohd-storage-server` debug binary (the script will run `cargo build` if it's missing).
- Built `ohd-connect` release binary at `connect/cli/target/release/ohd-connect` (the script will build it if missing).
- `pnpm` for the Care web app (`care/web/`).

The web app must already have `pnpm install` and `pnpm gen` run once. If not:

```sh
cd care/web
pnpm install
pnpm gen
```

## Running the demo

In **terminal A** (driver):

```sh
bash care/demo/run.sh
```

This boots the storage server, seeds 5 events, issues the grant token, and prints a URL like:

```
  http://localhost:5173/?token=ohdg_<base32>
```

In **terminal B** (web dev server):

```sh
cd care/web
pnpm dev
```

In **the browser**: paste the URL from terminal A.

## Expected UI behaviour

After the bootstrap WhoAmI + initial QueryEvents lands:

- **Roster** — one patient card; the slug is `/patient/patient`. The label is `Patient <ulid-prefix>`.
- **Patient header** — read-scope `std.blood_glucose, std.heart_rate_resting, std.body_temperature, std.medication_dose, std.symptom`; write-scope `std.clinical_note`; approval-mode `every write queues for patient approval`.
- **Vitals tab** — sparklines for glucose (2 readings: 120 + 138), heart rate (72 bpm), temperature (36.7°C).
- **Symptoms tab** — one row "mild headache" severity 1.5/5 (the CLI's 0–10 scale gets mapped to 1–5).
- **Notes tab** — empty.

Click **"+ New note"** on the Notes tab, type any text, submit, confirm. The toast says *"Submitted to <label> — awaiting patient approval."* The note does NOT appear on the Notes tab yet (because the grant's approval_mode is `always`, the write went to `pending_events`).

## Approving the pending note

In **terminal A** (after the demo script's tail printed the seed log):

```sh
# In a third terminal — let the script keep tail-ing the server log.
ohd-storage-server pending-list --db /tmp/ohd-demo.db
```

You'll see something like:

```
ULID                        STATUS    GRANTEE                   EVENT_TYPE                SUBMITTED_AT
01HVK3X1Y2Z…                pending   Dr. Smith                 std.clinical_note         1715210000000
```

Approve it:

```sh
ohd-storage-server pending-approve --db /tmp/ohd-demo.db --ulid 01HVK3X1Y2Z…
```

Reload the browser. The clinical note now appears on the Notes tab with status `committed`.

## Troubleshooting

- **"No grant token" page on first load** — the URL didn't include `?token=…`. Re-paste from the script's output.
- **"Could not load from storage"** — the storage server isn't running on `:18443`, or the grant token has expired (default 30 days). Re-run `bash care/demo/run.sh`.
- **CORS error in the browser console** — the storage server is running without the CORS layer (`--no-cors` flag, or an old build). The script doesn't pass `--no-cors`, so re-check the running pid.
- **`pnpm gen` never run** — the generated files at `care/web/src/gen/ohdc/v0/` are missing. Run it first.
- **Connect CLI complains about "no storage URL"** — the script passes `--storage` and `--token` flags explicitly so this shouldn't happen. If it does, your CLI binary may be older than the storage protocol; rebuild with `cd connect/cli && cargo build --release`.

## What's stubbed

The demo exercises the happy path of write-with-approval. These pieces are still TODO and don't ship in this pass:

- **Real `ListPending` / `ApprovePending` RPCs** on the storage server. The demo uses tactical CLI subcommands (`pending list` / `pending approve`) that operate directly on SQLite. The OHDC RPCs are stubbed (`Unimplemented`) per `storage/STATUS.md`.
- **Patient-side reject flow.** `pending reject` is not implemented as a CLI subcommand yet (it's the next-step pickup).
- **Care MCP** integration. The Care web app talks to OHDC directly; the MCP scaffold in `care/mcp/` is being switched to Python+FastMCP separately.
- **Multi-patient roster.** v0 = one grant = one patient. The vault + `switch_patient` MCP tool comes when Care holds N grants.

## Files

- `run.sh` — the driver script. Idempotent within a single run (cleans up the server pid on exit).
- `.last-self-token` / `.last-grant-token` — written by the script; persisted between runs. Safe to delete.
- `.server.log` / `.server.pid` — written by the script while the server is up.

## Cleanup

```sh
rm -f /tmp/ohd-demo.db
rm -rf care/demo/.server.* care/demo/.last-*
```
