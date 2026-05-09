# OHD Care CLI — Implementation Status

> Status of `care/cli/` as of 2026-05-09. Pairs with `../STATUS.md` (the
> Care-component status) and `../SPEC.md` (the implementation contract).
>
> v0.3 update (2026-05-09): OHDC Python protobuf generation, imports,
> checked-in stubs, and service strings now use `ohdc.v0` / `ohdc/v0`.
>
> v0.3 update (2026-05-09): canonical query-hash + operator-side audit
> landed. Every read RPC computes the SPEC §7.3 query-hash before the
> wire call and records a JSONL audit row in
> `$OHD_CARE_HOME/operator_audit.jsonl`. The hash matches the TS
> implementation byte-for-byte; cross-language parity is asserted by
> `tests/test_canonical_query_hash.py` (loads the shared vectors at
> `care/web/src/ohdc/__golden__/query_hash_vectors.json`).
>
> v0.3 update (2026-05-09): the canonical query-hash module moved to
> the new `ohd-shared` workspace package
> (`packages/python/ohd-shared/`). The CLI's
> `ohd_care.canonical_query_hash` is now a re-export shim over
> `ohd_shared.canonical_query_hash`. The CLI's `ohdc_client.py`
> (synchronous httpx HTTP/2 client) is intentionally kept local — its
> shape differs from the async MCP clients (sync iter, per-request
> bearer rotation, OperatorAuditEntry stamping); consolidating it is a
> follow-up.

## Summary

`ohd-care` is the operator-side terminal interface to the OHD Care
component. It speaks OHDC over Connect-RPC (HTTP/2 / h2c via
`httpx[http2]`) using a per-patient grant token from a local file-backed
vault.

`uv sync && uv run pytest` is green (49 tests, of which 1 is the
integration test against a built `ohd-storage-server`).
`uv run ohd-care --help` prints the full command tree.

## What changed in this pass (auth + KMS work)

- **OAuth 2.0 Device Authorization Grant (RFC 8628)** is wired via the
  new `oidc-login` subcommand. Discovery (RFC 8414) is automatic with
  fallback to OpenID-Connect's `/.well-known/openid-configuration`.
  Hermetic tests under `tests/test_oidc_kms.py` cover the device flow
  (`authorization_pending` → `slow_down` → success), discovery, and
  refresh.
- **KMS-encrypted credential vault** (`src/ohd_care/kms.py`) replaces
  the v0.1 plaintext-mode-0600 storage. Three backends: `keyring` (OS
  Secret Service / Keychain / Credential Manager), `passphrase`
  (scrypt-derived AES-GCM key), `none` (passthrough — tests + legacy
  mode). Default is `auto`: try keyring, fall back to passphrase. The
  on-disk format is a JSON envelope around AES-GCM ciphertext; legacy
  plaintext-TOML credentials still load (back-compat).
- **Operator-subject audit header**: `OhdcClient` now sends
  `x-ohd-operator-subject: <oidc_sub>` on every request when the user
  has logged in via `oidc-login`. Storage ignores it today; the header
  is the integration point for the two-sided audit JOIN per
  `spec/docs/design/care-auth.md` "Two-sided audit".

## What's wired

### Session / roster

| Command | State | Notes |
|---|---|---|
| `ohd-care login --storage URL [--operator-token …] [--kms-backend …]` | Done | Writes encrypted `credentials.toml` envelope (AES-GCM under the OS keyring by default; mode 0600). Manual `--operator-token` still accepted for tests / legacy demos. |
| `ohd-care oidc-login --issuer URL --client-id ID [--scope SCOPES] [--storage URL] [--kms-backend …]` | Done | OAuth 2.0 Device Authorization Grant (RFC 8628). Discovers the AS via `.well-known/oauth-authorization-server` with fallback to `.well-known/openid-configuration`. Polls the token endpoint with proper `authorization_pending` / `slow_down` handling. Persists `(access_token, refresh_token, expires_at_ms, oidc_subject, oidc_issuer)` into the encrypted vault. |
| `ohd-care logout [--kms-backend …]` | Done | Clears tokens from the vault (keeps storage URL). Local-side only — server-side session revocation is TBD until storage exposes the OAuth `/auth/logout` RPC. |
| `ohd-care add-patient --label … --token ohdg_… [--storage-url …] [--cert-pin-sha256 …] [--scope-summary …] [--notes …] [--force]` | Done | Writes one TOML per patient under `~/.config/ohd-care/grants/<label>.toml` (mode 0600). First patient added becomes active by convention. |
| `ohd-care patients` | Done | Lists vault entries with active marker, ULID prefix, expiry, scope summary. |
| `ohd-care use <label>` | Done | Persists the active label to `active.toml`. |
| `ohd-care current` | Done | Prints active patient + scope. |
| `ohd-care remove-patient <label>` | Done | Drops the row + clears active pointer if it pointed at the removed entry. |

### Reads (against the active grant)

| Command | State | Notes |
|---|---|---|
| `ohd-care query <event-type> [time options] [--limit N]` | Done | Generic read — accepts FQN `<ns>.<name>` or one of the recognized aliases. |
| `ohd-care temperature [time options]` | Done | Convenience wrapper for `query std.body_temperature`. |
| `ohd-care glucose` / `heart-rate` / `medications` / `symptoms` / `notes` | Done | Same shape; aliases per SPEC §11. |

Time selectors: `--last-day`, `--last-week`, `--last-month`, `--last-72h`,
`--from ISO`, `--to ISO`. Mutually exclusive with each other.

### Writes (write-with-approval against the active grant)

All `submit *` commands echo `"Submitting to <patient label>"` before
sending and require an interactive confirm (`--yes` to skip). The
grant's `approval_mode` decides whether the result is committed or
queued; the CLI prints both outcomes verbatim.

| Command | State | Notes |
|---|---|---|
| `ohd-care submit observation --type ns.name --value V [--unit U]` | Done | Generic single-channel observation. |
| `ohd-care submit clinical-note [--text … \| <stdin>] [--about …]` | Done | Reads body from `--text` or stdin; default event type `std.clinical_note` (auto-seeded by storage's `issue-grant-token` helper). |
| `ohd-care submit lab-result --type … --value V [--unit U] [--reference-range R]` | Done | Stub event-type `std.lab_result` (deployment-registered). |
| `ohd-care submit measurement --type … --value V [--unit U]` | Done | Generic single-channel numeric. |
| `ohd-care submit prescription --drug NAME --dose D --dose-unit U` | Done | Default event-type `std.prescription` (deployment-registered). |

### Pending queue

| Command | State | Notes |
|---|---|---|
| `ohd-care pending list [--status …] [--limit N]` | Done | Calls `OhdcService.ListPending`. Storage scopes results to the operator's own grant. |
| `ohd-care pending show <pending-ulid>` | Done | Locates one entry from the operator's own queue and renders content + audit metadata. |

### Audit

| Command | State | Notes |
|---|---|---|
| `ohd-care audit [time + filter options]` | Connected, server-stub-aware | Calls `OhdcService.AuditQuery`. If storage returns `Unimplemented` (which it does today for grant tokens — see below), the CLI catches the typed error and exits with a clear "TBD: storage AuditQuery RPC currently returns Unimplemented" message rather than a Connect stack. |

## Files added / changed in this pass

```
care/cli/
├── STATUS.md                      (NEW — this file)
├── pyproject.toml                 (existed)
├── buf.gen.yaml                   (existed)
├── scripts/gen_proto.py           (touched — writes proper __init__.py
│                                   for ohdc_proto on regen)
├── src/ohd_care/
│   ├── __init__.py                (existed)
│   ├── cli.py                     (REPLACED — was scaffolding stubs)
│   ├── config.py                  (existed)
│   ├── credentials.py             (existed)
│   ├── grant_vault.py             (existed)
│   ├── ohdc_client.py             (existed)
│   ├── util.py                    (existed)
│   ├── ohdc_proto/                (generated; regenerate with
│   │                               `uv run python scripts/gen_proto.py`)
│   └── commands/
│       ├── __init__.py            (touched — registers audit module)
│       ├── login.py               (existed)
│       ├── patients.py            (existed)
│       ├── query.py               (NEW)
│       ├── submit.py              (NEW)
│       ├── pending.py             (NEW)
│       └── audit.py               (NEW)
└── tests/
    ├── __init__.py                (existed)
    ├── test_smoke.py              (touched — pruned obsolete stubs)
    ├── test_cli.py                (NEW — behaviour tests; fixture
    │                               isolates `OHD_CARE_HOME`)
    └── test_integration.py        (NEW — boots a real
                                     `ohd-storage-server`, full round-trip)
```

## What's stubbed / TBD

### Storage-side blockers (CLI is correctly wired, but the wire returns "unimplemented")

- **`OhdcService.AuditQuery`** — currently returns `Unimplemented` for
  grant tokens. The CLI surfaces this with a typed `OhdcUnimplementedError`
  (caught + rendered as a "TBD: storage AuditQuery RPC currently returns
  Unimplemented" message + exit code 2). Re-test the `audit` command once
  the storage-side handler ships.
- **Open registry for clinical event types** — `std.observation`,
  `std.lab_result`, `std.prescription`, `std.referral`, etc. aren't in
  `migrations/002_std_registry.sql`. Storage's `issue-grant-token` helper
  auto-seeds `std.clinical_note` for the demo, but the rest need
  per-deployment registry rows. The CLI accepts the types verbatim;
  `submit` will get back a Connect error `not_found` if the type isn't
  registered. Document at deploy time.

### Care-side TBD (per SPEC §11 + §2)

- **OIDC operator login.** Done in this pass — `oidc-login` runs the
  Device Authorization Grant against the clinic's OIDC provider
  (Storage's own AS, Google Workspace, Microsoft Entra, Okta,
  Keycloak, Authentik). Storage today still ignores the operator
  token on the wire (only patient grants do work in OHDC calls), so
  the operator session is recorded locally and the CLI uses the
  grant-token bearer with the operator's `oidc_subject` attached as
  the `x-ohd-operator-subject` header. The integration is wire-ready;
  storage's two-sided-audit join lands when storage's
  operator-binding work ships.
- **KMS-encrypted credentials vault.** Done in this pass — the
  operator-credentials file is now an encrypted envelope. Per-patient
  grant-token files (`grants/<label>.toml`) are still plaintext mode
  0600 — that's the next step. See `grant_vault.py`'s `TODO(kms)`
  marker; rolling the same KMS backend through `GrantVault.save` /
  `.load` is mechanical now that the abstraction lives in `kms.py`.
  Multi-grant Care use should land on the encrypted path as soon as
  it touches v0.x patient volumes.
- **Federation when the patient's storage is on a different host.** The
  CLI does honor `PatientGrant.storage_url` per-grant (so multi-host
  vaults work); what's missing is the rendezvous URL handling for
  relay-mediated patients (`spec/relay-protocol.md`) and the
  `cert_pin_sha256` enforcement (the grant TOML stores it but the
  `httpx.Client` doesn't pin yet — defaults to system trust).
- ~~**Two-sided audit (`care_operator_audit`).** Per SPEC §7.2 the
  operator side records `(ts, operator_id, grant_id, action, query_hash,
  result, …)`.~~ Landed 2026-05-09. `src/ohd_care/operator_audit.py`
  writes one JSONL row per OHDC RPC under
  `$OHD_CARE_HOME/operator_audit.jsonl` (rolling 1000-entry buffer);
  `src/ohd_care/canonical_query_hash.py` provides the canonical
  `query_hash` that joins to the patient-side audit row per §7.3. Both
  modules mirror the TS source-of-truth in
  `care/web/src/ohdc/canonicalQueryHash.ts` byte-for-byte;
  `tests/test_canonical_query_hash.py` loads the shared golden vectors
  and asserts identicality.
- **Case operations.** SPEC §4 / §10.5 `open_case` / `close_case` /
  `handoff_case` / `list_cases` aren't surfaced. The OHDC service
  exposes them (`CreateCase` / `CloseCase` / `ListCases` / etc.), but
  grant-token authorization for case ops is one of the open spec
  questions. Add when the storage side is settled.
- **Approval / rejection of pending events.** Today `pending list` /
  `pending show` are read-only. Approve / reject is patient-side
  (Connect) — the operator does NOT approve their own submissions.
  Documented for posterity; this is by design.
- **Cohort / population queries.** Per SPEC §13 — out of scope for v1.
- **Localization.** English only per SPEC §13.

## How to verify

```sh
cd care/cli
uv sync                                        # one-time
uv run python scripts/gen_proto.py             # one-time after sync
uv run pytest -m "not integration"             # 37 tests, ~0.2 s
uv run ohd-care --help                         # prints the full tree

# Integration round-trip (needs `ohd-storage-server` built):
(cd ../../storage && cargo build -p ohd-storage-server)
uv run pytest -m integration                   # 1 test, ~1 s
```

## Manual smoke (round-trip)

```sh
# In storage/:
SERVER=$(pwd)/target/debug/ohd-storage-server
$SERVER init --db /tmp/ohd-cli.db
SELF=$($SERVER issue-self-token --db /tmp/ohd-cli.db --label dev)
GRANT=$($SERVER issue-grant-token --db /tmp/ohd-cli.db \
    --read std.blood_glucose,std.heart_rate_resting,std.body_temperature,std.symptom,std.clinical_note \
    --write std.clinical_note --approval-mode always \
    --label "Dr. Test" --expires-days 1)
$SERVER serve --db /tmp/ohd-cli.db --listen 127.0.0.1:18443 &

# In care/cli/:
export OHD_CARE_HOME=/tmp/ohd-care-demo
mkdir -p $OHD_CARE_HOME
uv run ohd-care login --storage http://127.0.0.1:18443
uv run ohd-care add-patient --label demo --token "$GRANT"
uv run ohd-care patients
uv run ohd-care use demo
echo "Patient reports headache resolved." | uv run ohd-care submit clinical-note --about test --yes
uv run ohd-care pending list
uv run ohd-care query glucose --last-day
```

The clinical note will appear in `pending list` (the demo grant has
`approval_mode=always`); approve it from the patient side via OHD Connect
or, for a CLI-only demo, with the storage server's tactical
`pending-approve --ulid <ulid>` subcommand.
