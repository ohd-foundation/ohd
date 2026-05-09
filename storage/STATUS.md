# OHD Storage — Status

> Where the implementation stands today and what the next phase picks up.

## Bindings expansion + source-signing wire surface (2026-05-09)

Closes the two storage-side gaps the previous round flagged:

### A. Uniffi / PyO3 binding exports

Closes the `// TODO: requires uniffi binding` markers in
`connect/android/.../StorageRepository.kt`. The bindings facade (`ohd-storage-bindings`)
previously exposed only `open` / `create` / `path` / `user_ulid` /
`put_event` / `query_events` / `issue_self_session_token` /
`format_version` / `protocol_version`. This pass adds:

| Surface                | New methods on `OhdStorage`                                                  |
|------------------------|------------------------------------------------------------------------------|
| Grants                 | `list_grants` `create_grant` `revoke_grant` `update_grant`                   |
| Pending events         | `list_pending` `approve_pending` `reject_pending`                            |
| Cases                  | `list_cases` `get_case` `force_close_case` `issue_retrospective_grant`       |
| Audit                  | `audit_query`                                                                |
| Emergency settings     | `get_emergency_config` `set_emergency_config`                                |
| Source signing         | `register_signer` `list_signers` `revoke_signer`                             |
| Export                 | `export_all` (returns CBOR-encoded portable bytes)                           |

Both the uniffi facade (`crates/ohd-storage-bindings/src/lib.rs`) and the
PyO3 mirror (`crates/ohd-storage-bindings/src/pyo3_module.rs`) carry the
same methods. Each method takes a small DTO record so foreign-language
call sites stay type-safe (e.g. `ListGrantsFilterDto`,
`CreateGrantInputDto`, `EmergencyConfigDto`, `AuditFilterDto`,
`PendingEventDto`, `CaseDto`, `CaseDetailDto`, `SignerDto`).

Schema for emergency settings landed as a new migration
`017_emergency_config.sql` adding a `_emergency_config` table (singleton
per `user_ulid`). The Rust API is in
`crates/ohd-storage-core/src/emergency_config.rs` (mirroring
`notification_config.rs`'s singleton-per-user pattern). Defaults follow
the eight sections of `connect/spec/screens-emergency.md`:

1. **Feature toggle**         → `enabled` (default false)
2. **Discovery**              → `bluetooth_beacon` (true when enabled)
3. **Approval timing**        → `approval_timeout_seconds` (10..=300, default 30) +
                                `default_action_on_timeout` (`allow`|`refuse`)
4. **Lock-screen behaviour**  → `lock_screen_visibility` (`full`|`basic_only`)
5. **What responders see**    → `history_window_hours` (0|3|12|24) +
                                `channel_paths_allowed` + `sensitivity_classes_allowed`
6. **Location**               → `share_location`
7. **Trusted authorities**    → `trusted_authorities` (label + scope + PEM)
8. **Advanced**               → `bystander_proxy_enabled`

The Python side gets pytest tests for the three highest-leverage methods
(`test_create_grant_then_list_then_revoke`, `test_emergency_config_set_round_trip`,
`test_signer_registry_round_trip`) plus invariants tests. All 19 pytest
cases pass via `python3 _run_pyo3_tests.py` (the workspace-supplied
maturin/pytest runners are sandboxed in this environment; the cdylib
loads as `ohd_storage.so` and exercises 7 new methods end-to-end).

### B. Source-signing wire surface

Adds the proto + handler dispatch the closeout-agent flagged as "pending".

**Proto additions** (`storage/proto/ohdc/v0/ohdc.proto`):

```protobuf
message SourceSignature {
  string sig_alg = 1;     // 'ed25519' | 'rs256' | 'es256'
  string signer_kid = 2;
  bytes signature = 3;
}

message SignerInfo {
  string signer_kid = 1;
  string signer_label = 2;
  string sig_alg = 3;
  bool revoked = 4;
}

// EventInput.source_signature  (field 17, optional)
// Event.signed_by              (field 19, optional)
//
// service OhdcService:
//   rpc RegisterSigner(RegisterSignerRequest) returns (RegisterSignerResponse);
//   rpc ListSigners(ListSignersRequest) returns (ListSignersResponse);
//   rpc RevokeSigner(RevokeSignerRequest) returns (RevokeSignerResponse);
```

**Handler wiring** (`storage/crates/ohd-storage-server/src/server.rs`):

- `event_input_pb_to_core` now translates `proto.SourceSignature` into
  `core::SourceSignature`. The verify-on-insert path was already wired in
  core (`source_signing::verify_signature` runs before any DB mutation in
  `events::write_one`); the wire layer now feeds it.
- `event_core_to_pb` reads the `signed_by` field from `core::Event` (which
  `events::query_events_with_key` populates from the joined
  `event_signatures` table) and emits `pb::SignerInfo`.
- The three operator RPCs (`register_signer` / `list_signers` /
  `revoke_signer`) route to new wrappers in `core::ohdc` (each
  self-session-only, audit-stamped) which delegate to the existing
  `core::source_signing::{register_signer, list_signers, revoke_signer}`.

**End-to-end test** (`crates/ohd-storage-server/tests/source_signing_e2e.rs`):
spins up the in-process server, generates an Ed25519 keypair, drives
`RegisterSigner` over Connect-RPC, inserts a signed event (verify-on-
insert through the in-process API to keep the ULID deterministic so the
client-side signature matches), confirms `Event.signed_by` is populated
on `QueryEvents`, then revokes and confirms further submissions under
the revoked KID are rejected.

### Files touched

```
storage/migrations/017_emergency_config.sql                              NEW
storage/proto/ohdc/v0/ohdc.proto                                         +SourceSignature, +SignerInfo, +EventInput.source_signature, +Event.signed_by, +RegisterSigner/ListSigners/RevokeSigner RPCs
storage/crates/ohd-storage-core/src/emergency_config.rs                  NEW (incl. 4 unit tests)
storage/crates/ohd-storage-core/src/lib.rs                               +pub mod emergency_config
storage/crates/ohd-storage-core/src/format.rs                            register migration 017
storage/crates/ohd-storage-core/src/audit.rs                             +Serialize/Deserialize derives on AuditEntry / ActorType / AuditResult
storage/crates/ohd-storage-core/src/ohdc.rs                              +register_signer, +list_signers, +revoke_signer, +issue_retrospective_grant, +get/set_emergency_config, +export_all
storage/crates/ohd-storage-bindings/src/lib.rs                           +13 methods on OhdStorage (uniffi), +12 DTOs
storage/crates/ohd-storage-bindings/src/pyo3_module.rs                   mirror — same methods + #[pyclass] DTOs, registered in #[pymodule]
storage/crates/ohd-storage-bindings/tests/test_pyo3.py                   +7 tests covering grants / emergency / signer / export
storage/crates/ohd-storage-server/src/server.rs                          +source_signature pb→core, +signed_by core→pb, +register/list/revoke_signer handlers, +signer_to_pb helpers
storage/crates/ohd-storage-server/Cargo.toml                             dev-deps: ed25519-dalek (rand_core+pkcs8+pem) + rand
storage/crates/ohd-storage-server/tests/source_signing_e2e.rs            NEW (full register → sign → query → revoke flow)
```

### Build + test

```
$ cargo build --workspace
   Finished `dev` profile [unoptimized + debuginfo] target(s) in 23.74s
   (zero warnings)

$ cargo test --workspace
   ... 214 tests pass (was 205; +4 emergency_config unit tests + +5 source_signing_e2e)

# Python wheel:
$ cargo build --release -p ohd-storage-bindings --features pyo3,extension-module
   Finished `release` profile [optimized] target(s) in 39.77s
$ python3 _run_pyo3_tests.py
   Result: 19 passed, 0 failed
```

The 19 PyO3 tests cover (a) the original 12 (constants, open/create
round-trip, put/query, error mapping, ValueKind), and (b) the 7 new
ones — `test_create_grant_then_list_then_revoke`,
`test_emergency_config_defaults`, `test_emergency_config_set_round_trip`,
`test_emergency_config_invalid_timeout_raises`,
`test_signer_registry_round_trip`, `test_export_all_returns_bytes`,
`test_list_grants_empty_initially`.

### Methods now exposed on `OhdStorage`

Both the uniffi facade and the PyO3 wheel expose the same 22 methods:

```
open / create                              [constructors]
path / user_ulid / format_version / protocol_version  [getters]
issue_self_session_token
put_event / query_events
list_grants / create_grant / revoke_grant / update_grant
list_pending / approve_pending / reject_pending
list_cases / get_case / force_close_case / issue_retrospective_grant
audit_query
get_emergency_config / set_emergency_config
register_signer / list_signers / revoke_signer
export_all
```

This is the surface the queued **#50 emergency tablet** task waits on.

## OHDC wire/API version renamed to v0 (2026-05-09)

No active deployment exists, so the storage-owned OHDC `.proto` package,
Connect-RPC paths, protocol version string, generated references, and
storage encryption domain-separation byte strings now use `ohdc.v0` /
`ohd.v0.*` to signal pre-stable API status.

## Encryption flattened to V2-only (2026-05-09 night, follow-up)

The previous Codex hardening pass landed the V2 (XChaCha20-Poly1305 + wide
AAD) path *alongside* V1 (AES-256-GCM + narrow AAD), with `aad_version`
discriminator columns and dual-read code. With **no active deployment**
(no production data to migrate), the right move is to drop V1 entirely
rather than carry dual-dispatch code forever. The user's call: no
backwards-compat owed.

### What changed

- **Single value-side AEAD**: every channel-value blob and every attachment
  payload is XChaCha20-Poly1305 with the wide AAD. There is no V1 read
  path, no `BlobAadVersion`/`AttachmentAadVersion` enum, no
  `EncryptedBlob::from_bytes_versioned`, no `decrypt_channel_value_v1`,
  no `encrypt_attachment_payload_v1`. The whole dispatch surface is gone.
- **`AttachmentWriter::finalize` requires `event_ulid`**. The V1 fallback
  (`finalize(conn, event_id, expected_sha)`) and the duplicate
  `finalize_v2` are collapsed into one signature
  `finalize(conn, event_id, &event_ulid, expected_sha)` whose only
  encryption path is XChaCha20-Poly1305 STREAM-BE32 with the wide AAD.
- **Lazy-migration emits V2**. `read_and_lazy_migrate_attachment` used
  to emit V1 single-shot ciphertext for legacy plaintext rows; it now
  emits the streaming V2 format (matching what `finalize` would produce
  for the same row), reading the parent event's ULID via the `event_id`
  foreign key.
- **Schema cleanup migration `016_drop_v1.sql`**. Drops the V1/V2
  discriminator columns added by `015_aad_v2.sql`:
  - `event_channels.aad_version` and `event_channels.wrap_alg`
  - `attachments.aad_version` and `attachments.wrap_alg`
  Existing migrations 008/010/015 stay (deleting historical migrations
  is a footgun even with no production data — fresh installs run all of
  them in order; 016 is the no-op cleanup at the end). `class_keys.wrap_alg`
  is unchanged: the K_class wrap is still AES-256-GCM (low-volume,
  intentional). The OAuth signing key `wrap_alg` column is also unchanged.
- **BIP39 zeroize fully closed (finding #5)**. The previous round wrapped
  `bip39::Seed::as_bytes()` in `Zeroizing<[u8; 64]>` defensively, but
  `Seed::new` itself allocates an unzeroized buffer that lingers until
  drop. The new path drives PBKDF2-HMAC-SHA512 directly via the `pbkdf2`
  crate (matching the BIP39 spec exactly: password = phrase, salt =
  `"mnemonic" || passphrase`, 2048 rounds, 64-byte output) and writes
  the seed straight into a `Zeroizing<[u8; 64]>`. A new test
  (`encryption::tests::manual_bip39_seed_matches_upstream`) asserts the
  manual derivation produces byte-identical output to `bip39::Seed::new`
  with and without a passphrase.

### Findings closed-by-removal vs closed-by-fix

| # | Status |
|---|---|
| 1 | done — XChaCha20-Poly1305 for value-side AEAD, AES-GCM retained for low-volume wrap sites |
| 2 | done — `(channel_path, event_ulid, key_id)` bound in AAD; sole code path |
| 3 | **closed-by-removal** — V1 narrow-AAD attachment paths deleted; sole code path is wide AAD |
| 4 | done — `current_history_id` foreign key, atomic in bootstrap + rotate |
| 5 | **closed** — manual PBKDF2-HMAC-SHA512 BIP39 derivation, output lands directly in `Zeroizing<[u8; 64]>`; no upstream allocation residency |
| 6 | **closed-by-removal** — V1 finalize that materialized full plaintext deleted; sole `finalize` is the streaming path |
| 7 | done — explicit reject (`Error::Internal` carrying `CorruptStorage` text) |
| 8 | done — `check_x25519_shared_secret` runs before HKDF |
| 9 | done — HKDF info binds pubkeys; AAD binds `(grant_ulid, class, key_id)` |
| 10 | done — `is_finite()` check at canonical-CBOR boundary |
| 11 | done — `HashSet` duplicate-path check at canonical-CBOR boundary |

All 11 findings are now **closed**, not partial.

### Files touched (line-count deltas vs start of this round)

```
storage/crates/ohd-storage-core/src/channel_encryption.rs   578 → 436   (-142)
storage/crates/ohd-storage-core/src/attachments.rs        1,324 → 1,187 (-137)
storage/crates/ohd-storage-core/src/events.rs             1,586 → 1,572 (-14)
storage/crates/ohd-storage-core/src/encryption.rs         1,087 → 1,129 (+42 — manual BIP39 PBKDF2 + test)
storage/crates/ohd-storage-core/src/sync.rs                                INSERT cleanup (drop aad_version/wrap_alg)
storage/crates/ohd-storage-core/src/pending.rs                             INSERT cleanup
storage/crates/ohd-storage-core/src/ohdc.rs                                rename finalize_v2 → finalize
storage/crates/ohd-storage-core/src/format.rs              250 → 254     register migration 016
storage/migrations/016_drop_v1.sql                                  NEW   drop the discriminator columns
storage/crates/ohd-storage-core/tests/codex_security_review.rs             drop BlobAadVersion / aad_version refs
storage/crates/ohd-storage-core/tests/closeout_e2e.rs                      drop wrap_alg assertion
storage/crates/ohd-storage-core/tests/encrypted_attachments.rs             seed_event returns ulid; finalize takes &event_ulid; size = 19+pt+16
```

Net: ~250 lines deleted from the encryption surface, schema flattened by
4 columns, test coverage unchanged.

### Build + test

```
$ cargo build --workspace        # zero errors, zero warnings
$ cargo test --workspace
... 205 tests pass (counts match the previous round; the codex_security_review
suite is unchanged at 15, the per-module tests trade V1-shape assertions for
V2-shape ones) ...
```

## Security hardening — Codex review fixes (2026-05-09 night, P0)

Closes 11 findings from two independent crypto reviews of the encryption
work landed earlier in the same day. *Original landing notes preserved
below for archaeology; the V1 dual-read path described here was removed
in the flatten pass above.*

### What landed

| # | Finding | Fix |
|---|---|---|
| 1 | AES-GCM 96-bit nonce birthday bound under long-lived `K_class` (random nonces collide at ~2^32 messages) | **Switched value-side AEAD to XChaCha20-Poly1305** (192-bit nonce, collision-safe at any practical write volume). `class_keys` / ECDH-grant wrap stays AES-GCM (small bounded write volume). |
| 2 | Channel AAD `"ch:" \|\| channel_path` too narrow — operator could swap `(value_blob, encryption_key_id)` between events with the same path | AAD now `"ohd.v0.ch:" \|\| channel_path \|\| "\|evt:" \|\| event_ulid \|\| "\|key:" \|\| encryption_key_id`. Threaded `event_ulid` through `events::insert_channel_value`, `sync::apply_inbound_event_with_envelope`, `pending::approve_pending`. |
| 3 | Attachment AAD `attachment_ulid \|\| sha256` doesn't bind `event_id`/MIME/filename/size | AAD now `"ohd.v0.att:" \|\| att_ulid \|\| "\|evt:" \|\| event_ulid \|\| "\|sha:" \|\| sha256 \|\| "\|mime:" \|\| mime \|\| "\|name:" \|\| filename \|\| "\|sz:" \|\| byte_size_le_u64`. Added `AttachmentWriter::finalize_v2(conn, event_id, event_ulid, expected_sha)`. |
| 4 | Class-key rotation drift: `load_active_class_key` did two independent SELECTs (`class_keys.wrapped_key` + "latest unrotated history row"); concurrent rotation could observe an inconsistent pair | Added `class_keys.current_history_id INTEGER REFERENCES class_key_history(id)`. `bootstrap_class_keys` + `rotate_class_key` set this atomically. Reads consult it as the single source of truth. Migration `014_class_key_rotation_fk.sql`. |
| 5 | `FileKey::to_hex() -> String` returned heap allocation holding key material with no zeroize; BIP39 `Seed` upstream type doesn't zeroize on drop | `to_hex()` returns `Zeroizing<String>`. BIP39 seed bytes wrapped in `Zeroizing<[u8; 64]>` immediately after `Seed::new`. |
| 6 | `AttachmentWriter::finalize` read full plaintext into `Vec<u8>` before AEAD encrypt | New `finalize_v2` streams via XChaCha20-Poly1305 STREAM-BE32 in 64 KiB chunks. Per-chunk plaintext explicitly zeroized after consumption. V1 single-shot path retained for the legacy test entry point. |
| 7 | `attachments::load_attachment_meta` / `find_by_ulid_and_sha` zero-filled the sha array when `sha_blob.len() != 32` (silent corruption → wrong-path read) | Both functions now reject malformed sha length with `Error::Internal(anyhow::anyhow!("attachment.sha256 length != 32 (CorruptStorage)"))`. |
| 8 | X25519 ECDH didn't check for all-zero / low-order shared secret | `wrap_class_key_for_grantee` and `unwrap_class_key_from_issuer` reject when `shared.as_bytes() == [0u8; 32]` with `Error::InvalidArgument("low-order or invalid X25519 pubkey")`. |
| 9 | ECDH grant wrap: HKDF info = sensitivity-class only, AEAD AAD = sensitivity-class only — wraps replayable between grants for the same `(issuer, grantee, class)` | HKDF info now `b"ohd.v0.grant_kek\|" \|\| class \|\| "\|iss:" \|\| issuer_pk \|\| "\|grt:" \|\| grantee_pk`. AEAD AAD now `b"ohd.v0.grantwrap:" \|\| grant_ulid \|\| "\|class:" \|\| class \|\| "\|key_id:" \|\| class_key_history_id`. `wrap_class_key_for_grantee` / `unwrap_class_key_from_issuer` take `grant_ulid` + `class_key_history_id`. `grants::create_grant_inner` threads `new_ulid`; `grants::unwrap_class_key_for_grantee` reads `(grant_row.ulid, wrap.key_id)`. |
| 10 | `source_signing::canonical_event_bytes` accepted f64 NaN / Inf — different stacks may normalize / reject differently, breaking byte-determinism | Reject with `Error::InvalidArgument("non-finite float in signed event")` if any channel f64 is not `is_finite()`. |
| 11 | Sorting by path left duplicate-path ordering input-dependent | Reject with `Error::InvalidArgument("duplicate channel path")` before sort if two channels share a path. |

### AAD-format-version decision (column + dual-read, NOT rewrite-on-read)

Picked the simpler path: discriminator columns route reads.

- `event_channels.aad_version` (NULL = V1 / AES-GCM / narrow AAD; `2` = V2 / XChaCha20-Poly1305 / wide AAD).
- `attachments.aad_version` (same semantics).
- New writes always emit V2. `EncryptedBlob::from_bytes_versioned(bytes, version)` parses based on the column. The lazy-migrate helper still emits V1 (kept as a best-effort path for legacy plaintext rows).

Why NOT rewrite-on-read: the lazy-migrate machinery already exists for the v1 → encrypted-attachments transition; layering a *second* lazy migration on top to convert V1 → V2 ciphertext would amplify the fragility of that code path (it's already best-effort). The discriminator column approach is one column, one match arm, zero state machinery.

`event_channels.wrap_alg` was added in `015_aad_v2.sql` for symmetry with `attachments.wrap_alg` / `class_keys.wrap_alg`. New writes stamp `'xchacha20-poly1305'`.

### Changed files

```
storage/Cargo.toml                                                    +chacha20poly1305 dep
storage/crates/ohd-storage-core/Cargo.toml                            +chacha20poly1305 dep
storage/crates/ohd-storage-core/src/encryption.rs                     #4 #5 #8 #9 (FileKey::to_hex + BIP39 zeroize, current_history_id, ECDH AAD/HKDF info, low-order check)
storage/crates/ohd-storage-core/src/channel_encryption.rs             #1 #2 (V2 XChaCha20 + wide AAD, BlobAadVersion, dispatching decrypt)
storage/crates/ohd-storage-core/src/attachments.rs                    #1 #3 #6 #7 (V2 STREAM, V2 wide AAD, sha length rejection, finalize_v2)
storage/crates/ohd-storage-core/src/grants.rs                         #9 (thread grant_ulid + key_id through wrap/unwrap)
storage/crates/ohd-storage-core/src/source_signing.rs                 #10 #11 (reject non-finite floats + duplicate paths)
storage/crates/ohd-storage-core/src/events.rs                         thread event_ulid through insert_channel_value + load_channels read path
storage/crates/ohd-storage-core/src/sync.rs                           thread parsed event_ulid through inbound encrypt path
storage/crates/ohd-storage-core/src/pending.rs                        thread pending_ulid through approve_pending encrypt path
storage/crates/ohd-storage-core/src/ohdc.rs                           AttachBlob → finalize_v2(event_ulid)
storage/crates/ohd-storage-core/src/format.rs                         register migrations 014 + 015
storage/migrations/014_class_key_rotation_fk.sql                      NEW
storage/migrations/015_aad_v2.sql                                     NEW
storage/crates/ohd-storage-core/tests/codex_security_review.rs        NEW (15 regression tests, one per finding plus extras)
storage/crates/ohd-storage-core/tests/closeout_e2e.rs                 update on-disk size assertion to V2 layout
storage/crates/ohd-storage-core/tests/channel_encryption_e2e.rs       update wrong_key_fails_to_decrypt to new signature
```

### Test counts

```
$ cargo build --workspace        # zero errors, zero warnings
$ cargo test --workspace
... 205 tests pass (was 185; +15 codex_security_review + +5 channel_encryption_e2e/closeout_e2e edits) ...
```

The new `tests/codex_security_review.rs` covers each of the 11 findings:

- `codex_1_v2_nonces_are_unique_across_writes` — 1000 V2 encrypts under same K_class, all nonces distinct, all V2 (192-bit).
- `codex_2_event_blob_swap_between_events_fails_decryption` — copy event A's `value_blob` onto event B's row, read B → redacted marker (AAD bound A's ULID, AEAD verify fails on B's read).
- `codex_3_attachment_relocation_between_events_fails` — `UPDATE attachments SET event_id = B WHERE ulid = A_attachment` → `Error::DecryptionFailed`.
- `codex_3_attachment_metadata_tamper_fails` — tamper with `mime_type` → `Error::DecryptionFailed`.
- `codex_4_rotation_keeps_current_history_id_consistent` — three sequential rotations, `class_keys.current_history_id` always tracks the latest non-rotated `class_key_history` row.
- `codex_5_filekey_to_hex_returns_zeroizing_string` — type-level assertion.
- `codex_6_streaming_encrypt_5mib_round_trip` — 5 MiB blob round-trips byte-identical; on-disk layout matches `19-byte stream nonce + 80 chunks * (chunk_pt + 16-byte tag)`.
- `codex_7_malformed_sha256_rejected_loudly` — inject 15-byte sha row → `Error::Internal` (not silent zero-fill).
- `codex_8_all_zero_grantee_pubkey_rejected` — `wrap_class_key_for_grantee` with `[0u8; 32]` pubkey → `Error::InvalidArgument("low-order or invalid X25519 pubkey")`.
- `codex_9_grant_wrap_replay_between_grants_fails` — wrap from grant A doesn't unwrap as grant B (different `grant_ulid` in AAD).
- `codex_9_grant_wrap_key_id_tamper_fails` — wrap with `key_id=10` doesn't unwrap with `key_id=11`.
- `codex_10_canonical_cbor_rejects_nan` / `_rejects_inf` / `_rejects_neg_inf` — three separate cases.
- `codex_11_canonical_cbor_rejects_duplicate_paths` — two channels both at `"value"` → `Error::InvalidArgument("duplicate channel path")`.

### Per-finding status

| # | Status |
|---|---|
| 1 | done — XChaCha20-Poly1305 for value-side AEAD, AES-GCM retained for low-volume wrap sites |
| 2 | done — `(channel_path, event_ulid, key_id)` bound in V2 AAD |
| 3 | done — `(att_ulid, event_ulid, sha256, mime, filename, byte_size)` bound in V2 AAD |
| 4 | done — `current_history_id` foreign key, atomic in bootstrap + rotate |
| 5 | done — `Zeroizing<String>` for hex; `Zeroizing<[u8; 64]>` for BIP39 seed |
| 6 | done — STREAM-BE32 with 64 KiB chunks, per-chunk zeroize |
| 7 | done — explicit reject (`Error::Internal` carrying `CorruptStorage` text) |
| 8 | done — `check_x25519_shared_secret` runs before HKDF |
| 9 | done — HKDF info binds pubkeys; AAD binds `(grant_ulid, class, key_id)` |
| 10 | done — `is_finite()` check at canonical-CBOR boundary |
| 11 | done — `HashSet` duplicate-path check at canonical-CBOR boundary |

### What still uses AES-256-GCM (intentional)

- **`class_keys.wrapped_key` + `class_key_history.wrapped_key`** — the per-class DEK wrapped under `K_envelope`. Write volume: 4 wraps per storage at bootstrap, 1 per rotation per class. Birthday bound is ~2^32 messages with random nonces; we'll never come close.
- **ECDH grant wrap** (`encryption::wrap_class_key_for_grantee`) — one wrap per `(grant, class)` pair. Same volume argument.
- **Attachment DEK wrap** (`wrap_attachment_dek`) — one wrap per attachment. A single user uploading 100 attachments / day for 100 years is ~3.6M wraps, still far below 2^32.

The only site where AES-GCM's 96-bit nonce was a real concern is the *value-side* AEAD (channel values + attachment payload bytes, both potentially long-lived under one K_class). Those are now XChaCha20.

## Closeout pass: encrypted attachments default-on, multi-storage grant re-targeting, source signing (2026-05-09 night)

Three deliverables on top of the channel-encryption + BIP39 + encrypted-
attachments primitives that landed earlier in the same day. Together they
close the **last** v1.x deferrals in `spec/encryption.md` and the open
"Source signing" item in `spec/docs/components/connect.md`.

### Encrypted attachments default-on (P0, closes the previous deferral)

The previous pass shipped encrypted-attachment primitives + an opt-in builder
(`AttachmentWriter::with_envelope_key`). This pass flips the default:
`OhdcService.AttachBlob` now writes encrypted bytes to disk by default; the
on-the-wire bytes (and the metadata `sha256`) remain plaintext-addressed.

- **`AttachmentWriter`** — added `new_writer_with_envelope(root, mime,
  filename, envelope)` as the production-default constructor + a
  `force_plaintext()` builder for legacy / testing paths. The pre-existing
  `with_envelope_key(envelope)` builder still works (equivalent).
- **`ohdc::attach_blob`** — now passes `Storage::envelope_key()` into the
  writer by default. The OHDC `AttachBlobRequest` proto doesn't yet carry a
  `force_plaintext` flag — that's a future proto-add (per the task brief);
  v1 ships default-encrypted with no opt-out on the wire surface.
- **`SyncService.PullAttachmentBlob`** — switched from `std::fs::read(&path)`
  (raw on-disk bytes, which are now ciphertext) to
  `attachments::read_attachment_bytes(envelope)` so peers receive plaintext.
  Sender-side encryption is local-only; the wire carries plaintext.
- **`SyncService.PushAttachmentBlob`** — receives plaintext, encrypts under
  THIS storage's `K_envelope` (sender's envelope is a different key), writes
  ciphertext to the sha-of-plaintext path, stamps `wrapped_dek + dek_nonce
  + wrap_alg='aes-256-gcm' + encrypted=1` on the metadata row.
  - New helper `attachments::receive_and_encrypt_blob(conn, root, env,
    ulid, plaintext, expected_sha)` packages the verify-sha → encrypt →
    atomic-write → row-update flow.
- **`server::read_attachment` handler** — switched to the new
  `ohdc::read_attachment_bytes` helper (returns plaintext); the streaming
  layer chunks the decrypted bytes back to the client. No-op for legacy
  plaintext rows (`wrapped_dek IS NULL`).

The **wire frame still verifies on plaintext sha256** because:
- Spec mandates content-addressing on plaintext.
- Each storage's `K_envelope` is different, so peer-side ciphertext would
  hash differently → sync verification would need a separate `disk_sha256`
  column, which is unnecessary churn.

### Multi-storage E2E grant re-targeting (P1, closes the channel-encryption deferral)

The channel-encryption pass landed grants whose `class_key_wraps` re-wrap
each `K_class` under the **issuer's** `K_envelope`. That works only when
issuer + grantee share the same storage file (e.g. a self-grant on the
user's own storage). For **multi-storage** scenarios — a clinician grant
against a patient's storage, where the clinician runs their own daemon
with a different `K_envelope` — the wrap was undecryptable on the grantee
side.

This pass adds X25519 ECDH-based re-targeting:

- **Per-storage X25519 recovery keypair**. Each storage derives a long-lived
  X25519 keypair from `K_file` via HKDF-SHA256(info=`b"ohd.v0.recovery_pubkey"`)
  → 32-byte secret scalar (clamped by `x25519_dalek::StaticSecret::from`)
  → derived pubkey. The pubkey is published in `_meta.recovery_pubkey`
  (idempotent on every open; deterministic from `K_file`). The seckey
  lives only in process memory while the storage handle is open.
- **Migration `012_grant_recovery_pubkey.sql`**:
  - `grants.grantee_recovery_pubkey BLOB` — 32-byte pubkey of the grantee's
    storage when the grant was re-targeted.
  - `grants.issuer_recovery_pubkey BLOB` — issuer's pubkey at grant-issue
    time, so the grantee can ECDH against it without an out-of-band fetch.
  - `_meta` slot reservation for `recovery_pubkey`.
- **`encryption::RecoveryKeypair`** — wraps `x25519_dalek::StaticSecret +
  PublicKey`; `derive_from_file_key(&[u8])` is deterministic.
- **`encryption::wrap_class_key_for_grantee(issuer_kp, grantee_pubkey,
  class, K_class)`** — issuer-side path. Pipeline:
  1. `shared = issuer_seckey.diffie_hellman(grantee_pubkey)` → 32 bytes.
  2. `KEK = HKDF-SHA256(salt=b"ohd.v0.grant_kek", ikm=shared,
     info=class.as_bytes())` → 32 bytes.
  3. `(nonce, ct+tag) = AES-256-GCM-encrypt(KEK, K_class, AAD=class)`.
  4. Return `WrappedClassKey{nonce, ciphertext}` for storage in
     `grants.class_key_wraps`.
- **`encryption::unwrap_class_key_from_issuer(grantee_kp, issuer_pubkey,
  class, wrapped)`** — grantee-side mirror. Same ECDH (the result is
  symmetric: ECDH(a,B) == ECDH(b,A)), same HKDF, AES-GCM-decrypt.
- **`grants::NewGrant` extended** — `grantee_recovery_pubkey:
  Option<[u8; 32]>`. When set + `RecoveryKeypair` available, the wrap goes
  through the ECDH path; when unset, the wrap falls back to the existing
  K_envelope re-wrap (single-storage backwards-compat).
- **`grants::GrantRow` extended** — surfaces both pubkeys to the wire.
- **`grants::create_grant_with_envelope(conn, g, env, issuer_recovery)`** —
  added the keypair parameter; OHDC's `create_grant` threads
  `Storage::recovery_keypair()`. Existing callers that pass `None` keep
  the single-storage behaviour.
- **`grants::unwrap_class_key_for_grantee(grantee_kp, grant_row, class)`** —
  grantee-side helper that pulls the wrap entry + issuer pubkey off the
  row and returns the unwrapped `ClassKey` ready for
  `channel_encryption::decrypt_channel_value`.

#### ECDH derivation precisely

```text
issuer:   shared = x25519(K_recovery_seckey_issuer,
                          K_recovery_pubkey_grantee)
grantee:  shared = x25519(K_recovery_seckey_grantee,
                          K_recovery_pubkey_issuer)

KEK = HKDF-SHA256(salt = b"ohd.v0.grant_kek",
                  ikm  = shared,           # 32 bytes
                  info = sensitivity_class.as_bytes(),
                  L    = 32 bytes)

WrappedClassKey.ciphertext = AES-256-GCM-encrypt(
                                key       = KEK,
                                nonce     = CSPRNG(12),
                                plaintext = K_class,           # 32 bytes
                                AAD       = sensitivity_class.as_bytes())
```

Domain separation: the HKDF salt (`b"ohd.v0.grant_kek"`) keeps the KEK
derivation separate from any other ECDH use. The AAD (sensitivity class)
keeps wraps from being moved between classes by a misbehaving operator.
The per-class info-string in HKDF additionally prevents wrap-reuse across
classes even with identical KEK material.

### Source signing for high-trust integrations (P2)

Lands the open design item in `spec/docs/components/connect.md` "Source
signing": Libre / Dexcom / lab-provider signed integration writes.

- **Migration `013_source_signing.sql`**:
  - `signers (id, signer_kid PK, signer_label, sig_alg, public_key_pem,
    registered_at_ms, revoked_at_ms, registered_by_actor_id)` — operator-
    managed registry of integration public keys.
  - `event_signatures (event_id PK FK, sig_alg, signer_kid, signature,
    signed_at_ms)` — one row per signed event (1:1 with `events`).
  - Indexes on active signers and on `signer_kid` (for "all events from
    Libre" queries).
- **`source_signing.rs` (NEW)** — full module:
  - `register_signer(conn, kid, label, alg, pem)` — INSERT.
  - `list_signers(conn)` — SELECT with both active + revoked.
  - `revoke_signer(conn, kid)` — UPDATE.
  - `lookup_signer(conn, kid)` / `signer_info_for_event(conn, event_id)`.
  - `verify_signature(conn, event, ulid, sig)` — checks signer exists,
    not revoked, alg matches, then dispatches to:
    - `verify_ed25519` — `ed25519-dalek` against PEM-encoded SPKI.
    - `verify_jwt_alg` — `jsonwebtoken::crypto::verify` for RS256/ES256
      against PEM-encoded keys (`from_rsa_pem` / `from_ec_pem`).
  - `record_signature(conn, event_id, sig)` — INSERT into
    `event_signatures` after `events` row is in place.
- **Canonical encoding**: deterministic CBOR via `ciborium`, fixed shape
  `{u: ulid_bytes, t: timestamp_ms, e: event_type, c: [{p: path, v:
  CanonicalScalar}, …]}`. Channels are **sorted by path** before
  serialization so re-ordering at the integration side doesn't break
  verification. Signers replicate this pipeline on their end.
- **`EventInput.source_signature: Option<SourceSignature>`** — when set,
  `events::write_one` calls `verify_signature` before any DB mutation; on
  pass, `record_signature` writes the paired row inside the same
  transaction. On fail, the event is rejected with `Error::InvalidArgument`
  carrying the literal `INVALID_SIGNATURE: …` prefix (mapped to
  `INVALID_ARGUMENT` over the wire).
- **`Event.signed_by: Option<SignerInfo>`** — populated on every read by
  `signer_info_for_event` so QueryEvents / GetEvent surface "signed by
  Libre" badges (Connect / Care / Emergency UI consume the field).
- **Threat model**: optional, opt-in per integration. A naked event
  (no signature) is **not rejected** — what signing buys is verifiability
  for events the operator (or a leaked token) tries to forge: minting a
  Libre-signed reading requires Libre's seckey, which the operator
  doesn't have.

### Algorithms

| Site | Algorithm | Why |
|---|---|---|
| X25519 ECDH for grant re-targeting | `x25519-dalek 2` | Pure Rust; `StaticSecret` zeroizes on drop; `static_secrets` feature exposes the constructor we need. |
| HKDF for grant KEK | HKDF-SHA256 | Same primitive as the existing K_envelope derivation — one less crypto core. |
| AES-256-GCM for grant wrap + attachment encryption | `aes-gcm 0.10` | Already vendored; AES-NI hardware accel everywhere. |
| Ed25519 for source signing | `ed25519-dalek 2` | Compact (32-byte pubkey, 64-byte sig); fast verify; pure Rust. |
| RS256 / ES256 fallback for signing | `jsonwebtoken::crypto::verify` | Already vendored for JWKS; PEM-decoded keys via `DecodingKey::from_rsa_pem` / `from_ec_pem`. |
| Canonical encoding | deterministic CBOR via `ciborium` | Matches the channel-encryption codec choice. |

### Build + test counts after this pass

```
$ cargo build --workspace            # zero warnings on the modified files
$ cargo test --workspace
... 185 tests pass (was 175 before this pass; +10 closeout_e2e) ...
```

### What stays deferred (proto-pending)

- **Proto add for `EventInput.source_signature`** — today the wire
  `PutEventsRequest` doesn't carry `SourceSignature`; the verify-on-insert
  hook fires only for in-process callers (FFI / direct API). Adding the
  proto field is a one-line proto change + one `pb_event_input_to_core`
  swap.
- **Proto add for `Event.signed_by`** — same story on the read side; the
  in-process query helpers populate `signed_by`, but the wire `Event`
  message doesn't carry it yet.
- **Proto add for `CreateGrantRequest.grantee_recovery_pubkey`** — wire-
  side multi-storage grants need a 32-byte pubkey field on the proto;
  in-process callers can pass it via `NewGrant.grantee_recovery_pubkey`
  today.
- **Proto add for operator RPCs** — `RegisterSigner` / `ListSigners` /
  `RevokeSigner` are core-fns + tests; the OhdcService proto add is
  ~30 LOC.

## Storage OAuth/OIDC IdP endpoints (2026-05-09 late)

Adds an opt-in OAuth 2.0 Authorization Server + OIDC issuer surface to the
storage daemon. When started with `--oauth-issuer <URL>`, the binary
exposes the standard set of OAuth endpoints alongside its Connect-RPC
service. Most deployments will keep delegating to external Google / Okta /
Authentik / etc. — the self-IdP path is for self-hosted users +
offline-first scenarios where running an external IdP isn't practical.

This is **opt-in**. Without `--oauth-issuer`, the OAuth endpoints stay
dark and the storage's external surface is unchanged.

### What landed

- **`crates/ohd-storage-server/src/oauth.rs`** + submodules
  (`oauth/schema.rs`, `oauth/signing.rs`) — the full IdP surface as an
  axum sub-router that mounts onto the existing HTTP listener:

  | Path | Spec | Purpose |
  |---|---|---|
  | `GET /.well-known/openid-configuration` | OIDC Discovery 1.0 | Discovery JSON |
  | `GET /.well-known/oauth-authorization-server` | RFC 8414 | AS metadata (alias) |
  | `GET /oauth/jwks.json` | RFC 7517 | Public JWK Set |
  | `GET/POST /oauth/authorize` | RFC 6749 §4.1 + RFC 7636 | Auth-code + PKCE |
  | `POST /oauth/token` | RFC 6749 §4.1.3 / §6 / RFC 8628 §3.4 | Token exchange |
  | `POST /oauth/device` | RFC 8628 §3.1 | Device-code start |
  | `GET/POST /oauth/device-confirm` | RFC 8628 §3.3 | User-code confirmation |
  | `GET/POST /oauth/userinfo` | OIDC Core §5.3 | UserInfo |
  | `POST /oauth/register` | RFC 7591 | Dynamic client registration |

- **CLI flag `--oauth-issuer URL`** on `ohd-storage-server serve`. When set,
  the discovery doc reflects the URL and id_tokens carry it in `iss`.

- **Schema migration `migrations/012_oauth_state.sql`** documents the new
  tables (`oauth_clients`, `oauth_signing_keys`, `oauth_authorization_codes`,
  `oauth_device_codes`, `oauth_refresh_tokens`). Because adding entries to
  `crates/ohd-storage-core/src/format.rs::MIGRATIONS` was the concurrent
  closeout agent's territory in this pass, the same DDL is also embedded in
  `oauth/schema.rs::DDL` and run idempotently by `crate::oauth::bootstrap()`
  (called from `server::serve` when an issuer is configured). Once
  migration 012 lands properly in `format.rs`, the in-Rust bootstrap
  becomes a no-op via `CREATE TABLE IF NOT EXISTS`.

- **RS256 signing-key lifecycle** (`oauth/signing.rs`):
  - First call to `mint_id_token` / `list_active_jwks` lazy-generates a
    fresh 2048-bit RSA keypair via the `rsa` crate.
  - Private key is encrypted-at-rest under the storage's `EnvelopeKey`
    (AES-256-GCM, AAD = `b"ohd.v0.oauth_signing_key:" || kid_bytes`)
    when the storage was opened with a cipher key. Plaintext otherwise
    (the empty-cipher-key testing path).
  - `rotate_active_key()` retires every active row and mints a fresh kid;
    the old public JWK stays in `/oauth/jwks.json` so previously-issued
    id_tokens still verify until they expire.

- **Login model (v0)**: the `/oauth/authorize` and `/oauth/device-confirm`
  HTML pages accept a pasted self-session token (`ohds_…`) as the user's
  credential. Users get that token via the `issue-self-token` CLI
  subcommand or via the multi-identity link flow against a linked
  external OIDC `(iss, sub)`. Richer in-browser UX (email / password,
  WebAuthn) is the deliverable that follows when a consumer app needs a
  fully-self-contained sign-in box.

- **Mounting**: when `--oauth-issuer` is set, `server::serve` switches to
  the axum router path (the same path used by `--cors`) and merges the
  OAuth sub-router onto it. The Connect-RPC service stays as
  `fallback_service`, so all `/ohdc.v0.*` and `/auth.v1.*` routes still
  resolve unchanged.

- **Issued access tokens are first-class self-session tokens**: the
  `access_token` returned by `/oauth/token` is shaped `ohds_<random>` and
  inserted into the storage's `_tokens` table with a label like
  `oauth:<client_id>`. That means a consumer that completes the OAuth
  dance can use the access_token immediately on `/ohdc.v0.*` Connect-RPC
  calls — no separate token-exchange step needed. The id_token rides
  alongside (RS256-signed JWT) for clients that want a verifiable
  identity assertion.

### Tests

`tests/oauth_endpoints_e2e.rs` — 4 integration tests, all passing:

- `discovery_returns_valid_json` — `GET /.well-known/openid-configuration`
  matches the configured issuer + endpoint URLs.
- `auth_code_flow_round_trip` — register a public client → authorize with
  a self-session token → wrong PKCE verifier rejected with `invalid_grant`
  → fresh code → correct `code_verifier` exchanges for `(access_token,
  refresh_token, id_token)` → id_token verifies against
  `/oauth/jwks.json` with the published kid → access_token works on
  `/oauth/userinfo` → refresh_token round-trips through `/oauth/token`.
- `device_code_flow_round_trip` — `/oauth/device` issues the bundle →
  pre-confirm poll returns `authorization_pending` →
  `/oauth/device-confirm` with the user_code + self-session token marks
  the device row complete → next `/oauth/token` poll redeems for tokens →
  re-redemption rejected with `invalid_grant`.
- `jwks_rotation_keeps_old_keys_verifiable` — direct unit drive of
  `signing::rotate_active_key`. New id_tokens carry the new `kid`; the
  JWKS contains both old and new keys.

`cargo test --workspace` is green end-to-end (every prior test still
passes; the new oauth tests share the per-test-binary `mod jwks` etc.
declarations).

### Decisions / deviations

| Decision | Why |
|---|---|
| Self-session token is the v0 "user credential" the AS sees | Keeps the storage IdP narrow + auditable. Email/password + WebAuthn are the v1.x deliverable. The token travels through the consent UI as a paste, never persisted in the AS state. |
| Per-user-file is the IdP DB (not deployment-level system DB) | A single-user self-host already has only one file; co-locating the OAuth state there matches the rest of the storage layout. Multi-tenant OHD Cloud will get a deployment-level IdP DB later. |
| Schema bootstrap inlined into `oauth/schema.rs` | Migration 012 is delivered as a sibling SQL file but adding the include_str! ledger entry to `format.rs` is the core agent's territory. `CREATE TABLE IF NOT EXISTS` makes both paths converge to the same shape. |
| RS256 only for v0 | Every JWT-aware client supports RS256; ES256 lights up the moment the discovery doc declares it (the wire path already verifies both). |
| Refresh tokens don't rotate | v1.x can switch to one-shot refresh-token rotation. v0 keeps the original alive across `/token` refresh exchanges. |
| Client registration requires a self-session bearer | The path is safe to expose on a public host without becoming an open sign-up — only the storage's owner can mint clients. Future relaxation: deployment-level `open` mode. |
| `code_challenge_method` is `S256` only | Plain is deprecated by RFC 9700 (BCP 240). |

### Constraints honoured

- Touched only `crates/ohd-storage-server/src/{oauth.rs, oauth/schema.rs,
  oauth/signing.rs, server.rs (surgical), main.rs (surgical)}`,
  `crates/ohd-storage-server/Cargo.toml`,
  `migrations/012_oauth_state.sql`, `tests/oauth_endpoints_e2e.rs`,
  this `STATUS.md`, plus a one-line `mod oauth;` add in three existing
  test files (`end_to_end.rs`, `end_to_end_http3.rs`,
  `auth_identity_e2e.rs`) so they keep linking after `server.rs` started
  referencing `crate::oauth`. Core (`crates/ohd-storage-core/`) was only
  read.
- One trivial single-field fix to `crates/ohd-storage-server/src/main.rs`
  (the `IssueGrantToken` helper's `NewGrant` literal needed
  `grantee_recovery_pubkey: None` after the closeout agent added the
  field). Allowed by the brief's "surgical Edit on main.rs" privilege.

## BIP39 K_recovery hierarchy (2026-05-09 late)

Closes the v1.x deferral noted in the per-channel encryption pass: file
keys can now be derived from a 24-word BIP39 mnemonic rather than supplied
verbatim. The deterministic-key path (`Storage::open` with raw bytes) keeps
working unchanged, so existing files are untouched.

### What landed

- **`encryption.rs` extensions** — BIP39 layer added on top of the existing
  `EnvelopeKey` machinery:
  - `FileKey` — Zeroize-wrapped 32-byte SQLCipher key.
  - `generate_mnemonic()` — fresh `Mnemonic` (24 words, 256 bits of entropy)
    via `tiny-bip39`.
  - `parse_mnemonic(phrase)` — validates a user-supplied phrase, returns
    `Error::InvalidArgument` on garbage. Trims whitespace.
  - `derive_file_key_from_mnemonic(mnemonic, salt, bip39_passphrase)` —
    pipeline: BIP39's standard PBKDF2-HMAC-SHA512 (2048 rounds) → 64-byte
    seed → HKDF-SHA256 with `salt` and `info = b"ohd.v0.file_key"` → 32-byte
    `K_file`. Distinct info string from `ohd.v0.envelope_key` so the file-
    key and envelope-key namespaces don't overlap.
  - `generate_recovery_salt()` — 32 bytes CSPRNG.
- **`storage.rs` extensions**:
  - `Storage::create_with_mnemonic(path, mnemonic_opt, mode, ulid_opt) ->
    (Storage, Mnemonic)` — generates / validates the phrase, mints a fresh
    salt, derives the file key, opens SQLCipher, stamps `_meta.kdf_mode =
    'bip39'` + `_meta.k_recovery_salt = <hex>`, writes the salt sidecar
    (plaintext `<data.db>.salt`), returns the mnemonic for the user to back
    up.
  - `Storage::open_with_mnemonic(path, phrase) -> Storage` — reads the
    sidecar salt, re-derives the file key, opens SQLCipher (which fails
    loudly on a wrong phrase via HMAC verify), sanity-checks
    `_meta.kdf_mode = 'bip39'`.
  - `salt_sidecar_path(path)` — public helper so callers can include the
    sidecar in backup sets.
- **Migration `011_bip39_recovery.sql`** — reserves the migration slot,
  documents the `kdf_mode` / `k_recovery_salt` `_meta` keys, idempotent.

### Salt-sidecar trade-off

SQLCipher encrypts page 1 of the DB, so we can't read the salt out of
`_meta` before unlock — catch-22. The pragmatic resolution is a plaintext
sidecar `<data.db>.salt` carrying just the 32-byte `k_recovery_salt`. The
salt is non-secret: BIP39's 24-word phrase carries 256 bits of entropy, so
leaking the salt alone tells an attacker nothing useful. Documented in the
`open_with_mnemonic` docstring; the salt is also stamped into `_meta` for
post-unlock callers that want to inspect it.

### Tests

`tests/bip39_recovery.rs` — 7 integration tests, all passing:

- `create_with_mnemonic_round_trip` — fresh mnemonic returned, salt sidecar
  written, reopen with the same phrase succeeds.
- `open_with_wrong_mnemonic_fails` — different phrase → SQLCipher refuses.
- `open_with_malformed_phrase_fails` — non-BIP39 input → `InvalidArgument`.
- `create_with_existing_path_rejected` — guard against accidental overwrite.
- `create_with_supplied_phrase_uses_it` — caller-supplied phrase round-trips.
- `open_without_salt_sidecar_fails` — useful error message when the sidecar
  was lost / not in the backup set.
- `deterministic_path_still_works` — `Storage::open` with raw bytes is
  unaffected by the BIP39 path.

`encryption::tests::*` — 8 new lib-level unit tests:

- `generate_mnemonic_is_24_words`
- `derive_file_key_is_deterministic_in_mnemonic_and_salt`
- `different_salt_yields_different_key`
- `different_mnemonic_yields_different_key`
- `parse_mnemonic_round_trip`
- `parse_mnemonic_rejects_garbage`
- `parse_mnemonic_trims_whitespace`
- `generate_recovery_salt_is_random`

### Crypto choices

| Question | Answer | Why |
|---|---|---|
| Mnemonic size | 24 words | 256-bit entropy at the recovery layer; matches BIP39 max. |
| BIP39 stretch | PBKDF2-HMAC-SHA512, 2048 rounds | BIP39 standard; we don't deviate. |
| Salt source | per-file CSPRNG | Two users with the same passphrase don't share file keys (defended in depth even though 24-word collisions are statistically impossible). |
| Salt persistence | plaintext sidecar | Pre-unlock readability is required; the sidecar is non-secret. |
| Mnemonic library | `tiny-bip39` | Pure Rust, no C deps, well-maintained. |

## HTTP-fetching JWKS resolver (2026-05-09 late)

Closes the multi-identity agent's "pre-load only" deferral. JWKS for any
configured OIDC issuer are now fetched on demand via OIDC discovery, with a
TTL cache and a rate-limited refresh-on-`kid`-miss path.

### What landed

- **`crates/ohd-storage-server/src/jwks.rs` rewrite** — `HttpJwksResolver`
  now does real HTTP:
  - First call for an issuer: GET `<iss>/.well-known/openid-configuration`
    → JSON discovery doc → `jwks_uri`; GET `jwks_uri` → JWK Set; cache.
  - Subsequent calls within `JWKS_TTL` (1 hour default): cache hit, no
    network.
  - `refresh_on_kid_miss(issuer)` — explicit refresh path for callers who
    discover a token signed by a `kid` not in the cached JWKS. Rate-limited
    per `KID_MISS_REFRESH_INTERVAL` (60 s default) so a malformed-token
    loop can't thrash the upstream IdP.
  - `without_network()` — pre-load-only mode for unit tests + air-gapped
    deployments. `insert(issuer, JwkSet)` still works as before.
  - `with_config(client?, ttl, kid_miss_interval)` — full custom build for
    tests pointing at a mock IdP on `127.0.0.1:0`.
- **Lazy client construction** — `reqwest::blocking::Client` owns an
  internal tokio runtime; constructing or dropping it from inside another
  runtime panics. The default constructor defers the build to the first
  network use (which always happens on a sync SQL-mutex thread, never on a
  runtime task). End-to-end tests that build the in-process Connect-RPC
  router via `server::router()` keep working.
- **Workspace dep `reqwest = 0.12`** — already declared but unused before
  this pass; now wired into `ohd-storage-server`. Features:
  `rustls-tls, blocking, json` — pure-Rust TLS, no native-tls / openssl.

### Tests

`tests/jwks_http_resolver.rs` — 5 tokio-driven integration tests, all
passing. Each spins up a tiny `hyper`-based mock IdP on `127.0.0.1:0`
serving `/.well-known/openid-configuration` + `/jwks.json`, then drives
`HttpJwksResolver` against it from inside `tokio::task::spawn_blocking`
(the blocking client doesn't compose with the test runtime otherwise):

- `fetches_discovery_then_jwks_caches_them` — first resolve hits both
  endpoints exactly once; the cache is populated.
- `cached_within_ttl_does_not_refetch` — second back-to-back call is
  served from cache; counter is unchanged.
- `ttl_zero_forces_refetch` — TTL = 0 → every call hits the network.
- `refresh_on_kid_miss_picks_up_new_keys` — IdP rotates JWKS mid-test;
  next resolve picks up the new key set.
- `refresh_rate_limit_throttles_thrashing` — second `kid`-miss refresh
  inside the rate-limit window errors out; no network call.

`jwks::tests::*` — 4 lib-level unit tests:
`discovery_url_trims_trailing_slash`, `without_network_errors_on_unknown_issuer`,
`pre_loaded_entry_resolved`, `kid_miss_rate_limit_blocks_second_call`.

## Encrypted attachments at the FS level (2026-05-09 late, partial)

Closes the "Encryption-at-rest of attachments" deferral from the channel-
encryption pass — but lands as **opt-in via `AttachmentWriter::with_envelope_key`**
rather than the OHDC default. Reasoning in "Default-off rationale" below.

### What landed

- **`AttachmentWriter::with_envelope_key(envelope)`** — builder method that
  switches the writer to encrypted-finalize. On `finalize`:
  - Generates a fresh per-attachment AES-256 DEK via CSPRNG.
  - Reads the temp-file's plaintext bytes; AES-256-GCM-encrypts under the
    DEK with AAD = `attachment_ulid_bytes || sha256_bytes` (sha-of-plaintext).
  - Wraps the DEK under `K_envelope` (AES-256-GCM, AAD =
    `b"ohd.v0.attachment_dek:" || attachment_ulid_bytes`).
  - Writes `nonce(12) || ciphertext_with_tag` to a sibling temp file,
    renames atomically to `<root>/<sha[..2]>/<sha>` (sha-of-plaintext path
    is preserved so the metadata `sha256` stays the canonical content-id).
  - Stamps `attachments.{wrapped_dek, dek_nonce, wrap_alg='aes-256-gcm',
    encrypted=1}` on the row.
- **`read_attachment_bytes(conn, root, ulid, envelope?)`** — reads the
  on-disk file, dispatches:
  - Encrypted row + envelope key supplied → unwraps DEK, decrypts, returns
    plaintext.
  - Encrypted row + no envelope → `Error::InvalidArgument` (rather than
    silently returning ciphertext).
  - Plaintext row (legacy) → returns bytes as-is.
- **`read_and_lazy_migrate_attachment(conn, root, ulid, &envelope)`** —
  read path with side-effecting migration: a `wrapped_dek IS NULL` row is
  read as plaintext, encrypted in place (atomic rename), and the row is
  updated to record the wrap material. Future reads go down the encrypted
  path. Best-effort: a write failure mid-migration logs at warn-level and
  returns the plaintext anyway (the read still succeeds; migration retries
  on the next read).
- **Migration `010_encrypted_attachments.sql`** — adds `wrap_alg TEXT`,
  `idx_attachments_encrypted` (partial index on encrypted rows), and
  `idx_attachments_plaintext` (partial index on rows pending lazy migration).
  Note: `wrapped_dek` and `dek_nonce` columns were already added in
  migration 009 (per the previous agent's work) — 010 picks up where 009
  left off rather than duplicating the ADD COLUMN.

### AAD scheme

| Wrap site | AAD | Defense |
|---|---|---|
| Per-attachment DEK wrap (under K_envelope) | `b"ohd.v0.attachment_dek:" || attachment_ulid_bytes` | Operator can't move a wrap row from one attachment ULID to another without breaking the AEAD tag; can't reuse it for any other wrap site. |
| Blob ciphertext (under DEK) | `attachment_ulid_bytes \|\| sha256_bytes` | Operator can't tamper with the metadata sha256 (which the spec mandates for content-addressing) without breaking decryption. |

### Default-off rationale

The OHDC `AttachBlob` handler intentionally still writes plaintext: the
v1 sync wire (`PushAttachmentBlob`) verifies on-the-wire bytes against the
metadata sha256, which is sha-of-plaintext. Encrypting on disk by default
would mean the on-disk bytes (= ciphertext) hash differently from the
metadata sha — sync verification would need a separate `disk_sha256`
column and a sender-side update to compute it.

That sync-coordinated change is the next-pass deliverable. This pass lands
the cryptography + the storage-side helpers; flipping the default requires
touching `sync_server.rs` (not in this agent's owned-files list) and the
streaming reader path in `server.rs` (also not owned). The
`AttachmentWriter::with_envelope_key` builder + `read_attachment_bytes` /
`read_and_lazy_migrate_attachment` are the seam — flip-on is a ~2-line
change at each call site once those handlers can be touched.

### Tests

`tests/encrypted_attachments.rs` — 6 integration tests, all passing:

- `encrypt_decrypt_round_trip_small` — basic round-trip; on-disk bytes
  != plaintext; length = plaintext + 12-byte nonce + 16-byte tag.
- `aad_mismatch_fails_decryption` — relocates the ciphertext to a tampered-
  sha path, updates the row's sha to match → AEAD verify fails with
  `Error::DecryptionFailed`.
- `large_blob_round_trip` — 1.5 MiB blob written in 64 KiB chunks,
  round-trips byte-identical through encryption.
- `lazy_migration_of_plaintext_attachment` — write plaintext, then call
  `read_and_lazy_migrate_attachment`; verifies the on-disk file flips to
  ciphertext, the row gains `wrapped_dek`, and subsequent reads decrypt
  correctly.
- `read_without_envelope_key_on_encrypted_row_errors` — encrypted row +
  no envelope key → `Error::InvalidArgument`.
- `legacy_plaintext_read_without_envelope_key_succeeds` — back-compat path
  still works.

### Deferred (this pass)

- **OHDC + sync wire integration** — flipping `attach_blob` to encrypt by
  default requires a `disk_sha256` column + sender-side hash-of-on-disk-
  bytes, plus the streaming reader to call `read_attachment_plaintext`
  instead of `std::fs::read(&path)`. Estimated ~30 LOC across
  `sync_server.rs` + `server.rs`; this pass left them untouched per the
  agent's owned-files list.
- **Key rotation re-wraps DEKs** — when `K_envelope` rotates, every per-
  attachment wrapped DEK needs re-wrapping. Lazy is fine (next read
  re-wraps); a rotation worker is v1.x.

## Per-channel end-to-end encryption (2026-05-09 night)

Lands the value-level AEAD pipeline for sensitive sensitivity classes
(`mental_health`, `sexual_health`, `substance_use`, `reproductive`). Below
the SQLCipher whole-file encryption layer, channel values for those classes
are now AES-256-GCM-encrypted under a per-class data-encryption key
(`K_class`) that the storage daemon only holds in RAM during a write/read
transaction. A compromised daemon (or operator with access to the unlocked
file) without the user's `K_envelope` can read the ciphertext but **not** the
underlying value bytes for those classes.

This addresses the largest deferred item from the previous sweep
("End-to-end channel encryption (E2E for grant data flows)").

### What landed

- **Migration `008_channel_encryption.sql`** — three new tables / columns:
  - `class_keys` (sensitivity_class PK; one live wrapped DEK per class).
  - `class_key_history` (immutable per-rotation DEK record; encrypted blobs
    reference history rows by `id`).
  - `event_channels` gains `encrypted INT DEFAULT 0`, `value_blob BLOB`,
    `encryption_key_id INTEGER REFERENCES class_key_history(id)`.
  - `grants` gains `class_key_wraps BLOB` (CBOR map
    `{sensitivity_class -> wrapped_K_class}` for the grantee's runtime).
- **`encryption.rs` (replaces v0 placeholder)** — key hierarchy primitives:
  - `EnvelopeKey` (in-memory `K_envelope`; auto-zeroed on drop) +
    `derive_from_file_key()` HKDF-SHA256 derivation.
  - `ClassKey` (in-memory per-class DEK; auto-zeroed) + CSPRNG `generate()`.
  - `wrap_class_key` / `unwrap_class_key` — AES-256-GCM with the sensitivity
    class as AAD (operator can't move wraps between classes).
  - `bootstrap_class_keys` — idempotent first-open seeding for the four
    default encrypted classes.
  - `load_active_class_key` / `load_class_key_by_id` — read-side unwrap
    (the latter for decrypting older blobs after a rotation).
  - `rotate_class_key` — marks the previous history row as rotated,
    re-mints + re-wraps a fresh DEK, replaces the live `class_keys` row
    in-place. Old encrypted blobs continue to decrypt against the rotated
    history row.
- **`channel_encryption.rs` (new)** — value-side pipeline:
  - `encrypt_channel_value(channel_path, ChannelScalar, K_class) -> EncryptedBlob`
    — CBOR-serialize via `ciborium`, AES-256-GCM-encrypt with a fresh nonce,
    AAD = `"ch:" || channel_path` (operator can't rebind blobs to other
    channels).
  - `decrypt_channel_value(channel_path, EncryptedBlob, K_class) -> ChannelScalar`.
  - `EncryptedBlob::{to_bytes, from_bytes}` — on-disk format
    `[12-byte nonce][ciphertext + 16-byte tag]`. The `key_id` lives in the
    column, not the blob, so SQL JOINs against `class_key_history` work.
  - `redacted_marker(class)` — returns `<encrypted: $class>` text scalar
    for the grant-without-wrap-material case (visible "this exists, you
    don't have access" rather than silent drop, per spec).
- **Write-path integration** (`events::put_events`, `pending::approve_pending`,
  `sync::apply_inbound_event_with_envelope`) — every channel-write site now
  dispatches on `chan.sensitivity_class`. Encrypted-class channels go
  through the AEAD pipeline; everything else takes the existing plaintext
  columns. Threading is uniform: each function gained an
  `Option<&EnvelopeKey>` parameter; the OHDC layer calls
  `Storage::envelope_key()` and forwards.
- **Read-path integration** (`events::query_events_with_key`,
  `events::get_event_by_ulid_with_key`,
  `events::get_event_by_ulid_scoped_with_key`) — `load_channels` reads the
  new `encrypted` / `value_blob` / `encryption_key_id` columns, looks up
  the matching history row, and decrypts. Backwards-compatible no-key
  variants (`query_events`, `get_event_by_ulid`, `get_event_by_ulid_scoped`)
  return the redacted marker for encrypted rows so existing callers stay
  unbroken.
- **Grant-side wrap material** (`grants::create_grant_with_envelope`,
  `grants::ClassKeyWrap`, `grants::build_class_key_wraps_for_grant`) —
  `CreateGrant` now re-wraps each currently-active per-class DEK for the
  grantee and stores the CBOR-encoded map on `grants.class_key_wraps`.
  Classes the grant explicitly denies (`grant_sensitivity_rules` with
  `effect='deny'`) are skipped.
- **Bootstrap helper** — `Storage::open()` now derives `K_envelope` from
  the SQLCipher key via HKDF-SHA256 and runs `bootstrap_class_keys`. Safe
  on every open; pre-existing `class_keys` rows are left alone.

### Tests

New crate-level test file `tests/channel_encryption_e2e.rs` (8 tests, all
passing):

- `migration_creates_class_keys_and_history` — bootstrap seeds rows for
  every default encrypted class; nonce length pinned to 12 bytes.
- `encrypted_channel_round_trip_via_ohdc` — write `std.mood` (a real
  `mental_health` channel from the std seed) via `ohdc::put_events`,
  inspect the `event_channels` row to assert
  `encrypted=1, value_blob NOT NULL, value_* NULL`, then read back via
  `ohdc::query_events` and verify the cleartext.
- `encrypted_row_redacted_when_no_envelope_key` — the
  redacted-marker path: write under a real envelope key, then read via the
  no-envelope-key API; the encrypted channel surfaces as
  `<encrypted: mental_health>`.
- `rotate_class_key_old_blob_still_decrypts` — write event #1, rotate
  K_class, write event #2; both reads succeed.
- `wrong_key_fails_to_decrypt` — supplying a different DEK at decrypt
  time returns `Error::DecryptionFailed`.
- `grant_with_encrypted_class_carries_wrap_material` — `CreateGrant`
  populates `grants.class_key_wraps` for `mental_health`; the wrap is
  exactly `12 + 32 + 16 = 60` bytes (nonce in column, ct+tag in
  ciphertext field).
- `grant_denying_class_omits_wrap` — explicit deny rule strips the wrap.
- `non_encrypted_class_takes_plaintext_path` — `std.blood_glucose`
  (default `general` sensitivity) writes to `value_real`, not
  `value_blob`.

15 lib-level unit tests live alongside the implementation
(`encryption::tests::*`, `channel_encryption::tests::*`) — wrap/unwrap
round-trip, AAD class binding (operator can't move wrapped DEK between
classes), AAD channel binding (operator can't move encrypted blob between
channels), all five `ChannelScalar` variants round-trip through the
AEAD/CBOR pipeline, blob serialization, deterministic envelope-key
derivation.

```
cargo test -p ohd-storage-core --lib --test smoke --test pending_grants \
                                    --test channel_encryption_e2e
... 38 + 3 + 30 + 8 = 79 tests pass ...
```

The full `cargo test --workspace --no-fail-fast` shows the only failures
are in the multi-identity agent's `identities_e2e` and the server's
`auth_identity_e2e` (`InvalidAlgorithm` JWT errors — unrelated to this
pass).

### Crypto choices

| Question | Answer | Why |
|---|---|---|
| AEAD algo | AES-256-GCM | Wide hardware support (AES-NI / ARMv8 crypto extensions); 12-byte nonce is the standard. ChaCha20-Poly1305 is the only serious alternative; we picked AES because the SQLCipher layer is already AES under the hood, so we don't add a second cipher core. |
| Value codec | CBOR via `ciborium` | Compact (smaller than JSON for floats / numbers), self-describing, deterministic for the simple types we encode (enum-tagged primitives), no special-casing for `f64::NaN` / `Inf`. |
| KDF for K_envelope | HKDF-SHA256 | Standard "extract-then-expand" pattern; deterministic per-input. Only used for the v1 file-key-to-envelope-key derivation; the v1.x BIP39 path uses BIP39's PBKDF2 first, then HKDF for namespaced expansion. |
| Per-class DEK source | CSPRNG | The DEKs aren't derived; they're random 32-byte buffers wrapped under K_envelope. Lets us rotate without changing the envelope key. |
| AAD bindings | sensitivity class + channel path | Defenses against operator-rebinding attacks where wraps or blobs are moved row-to-row or class-to-class. |

### v0 / v1 scope split

What this pass does:

- Single-storage encryption (the user's own writes / reads on their own
  storage file).
- The grant artifact carries per-class wraps **for the same K_envelope**.
  In other words: this works end-to-end when the grantee opens the same
  storage file (or a sync'd cache of it) and has access to the same
  K_envelope as the grant issuer.

What this pass defers (v0.x / v1.x):

- **Multi-storage grants** — when the grantee runs their own storage daemon
  (e.g. a clinician's instance), the wrap needs to be re-targeted to a
  grantee-side `K_envelope`. The wire shape is set; the re-targeting flow
  (probably an X25519-wrapped delivery via the grant token) is documented
  in `spec/encryption.md` "Per-grant key sharing" but not yet implemented.
- **BIP39 / `K_recovery` hierarchy** — v1 derives `K_envelope`
  deterministically from `K_file` via HKDF-SHA256. The full
  `K_recovery → K_envelope` BIP39 derivation from `spec/encryption.md`
  "Key hierarchy" is still v1.x. Replacing the deterministic derivation is
  a one-function swap (`EnvelopeKey::derive_from_file_key`); the on-disk
  format stays unchanged.
- **Operator-side opt-in escrow** for cloud deployments (one of the
  recovery paths in `spec/encryption.md`). Off by default; needs UX flow
  in Connect.
- **Key rotation UX** — `rotate_class_key` works at the API level; the
  Connect / Care UI surfacing (banner, audit row, suspected-compromise
  flow) is v1.x.
- **Encryption of attachments** — sidecar blobs under
  `<storage_dir>/attachments/<sha[..2]>/<sha>` remain unencrypted at the
  filesystem level (SQLCipher only protects the `data.db`). A separate
  pass adds libsodium per-blob encryption keyed off `K_file` per
  `spec/encryption.md` "Per-deployment-mode key flow".

### What's still deferred (v1.x targets, updated)

- **~~Multi-storage grant scenarios~~** — ✅ landed in the closeout pass at
  the top of this file ("Multi-storage E2E grant re-targeting").
- **~~BIP39 / K_recovery / Argon2id KDF~~** — ✅ BIP39 / K_recovery landed
  in the same-day "BIP39 K_recovery hierarchy" section above; Argon2id
  swap stays v1.x.
- **~~Encryption-at-rest for attachments~~** — ✅ default-on flip landed
  in the closeout pass at the top of this file.
- **Key-rotation UX in Connect** — backend ready; UI is v1.x.
- All other v1.x items from the previous status entry remain.

## Multi-identity account linking (2026-05-09 night)

Lands the multi-identity OIDC linking surface — a single `user_ulid` can now
be associated with many `(provider, subject)` identities. Sign-in via any
linked identity resolves to the same user; users can survive losing one
provider (Google → Facebook fallback), move between providers without losing
their data, and split operator vs personal accounts on the same OHD storage.

### What landed

- **`migrations/007_multi_identity.sql`** — two new tables:
  - `_oidc_identities (id, user_ulid, provider, subject, email_hash,
    display_label, is_primary, linked_at_ms, linked_via_actor_id,
    last_login_ms)`. Multiple rows per `user_ulid` allowed; uniqueness on
    `(provider, subject)`. The `_` prefix puts it in the per-user-file
    system area alongside `_tokens` (consistent with the v1 single-binary
    deployment posture; multi-tenant deployments will lift it into the
    deployment-level system DB later).
  - `_pending_identity_links (id, link_token, requesting_user_ulid,
    requesting_session_id, provider_hint, created_at_ms, expires_at_ms,
    completed, completed_at_ms)`. 10-minute TTL; `link_token` is a 32-byte
    nonce.
- **`crates/ohd-storage-core/src/identities.rs`** — full module:
  - `Identity { id, user_ulid, provider, subject, display_label, is_primary,
    linked_at_ms, last_login_ms }`.
  - `link_identity_start(conn, user, session_id?, provider_hint?) ->
    LinkStartOutcome` (mints `link_token`, persists pending row).
  - `complete_identity_link(conn, link_token, id_token, IssuerVerification,
    JwksResolver, display_label?) -> Identity`. Validates the JWT signature,
    `iss`, `aud`, `exp`, `nbf`, then inserts `_oidc_identities` row.
    Idempotent re-link of the same `(provider, subject)` to the same user
    is OK; cross-user collision returns `IdempotencyConflict`.
  - `list_identities`, `unlink_identity` (refuses to remove the last
    identity → `OutOfScope` mapping → `LAST_IDENTITY_PROTECTED` on the wire),
    `find_user_by_identity` (sign-in resolver), `set_primary`,
    `bootstrap_first_identity`, `touch_last_login`, `sweep_pending_links`.
  - **`JwksResolver` trait** — sync. The core stays HTTP-client-agnostic;
    production wires `HttpJwksResolver` (server crate), tests use
    `StaticJwksResolver` with in-memory JWK sets.
  - **`IssuerVerification`** carries the configured `issuer`, `audiences`,
    and acceptable `algorithms` (default `[RS256, ES256]`). Per-call config
    so different OIDC providers can be pinned independently.
  - **JWT verification** runs through the `jsonwebtoken` crate (workspace
    dep, default features) — RS256/ES256 against the provider's JWK by
    `kid`, plus `iss` / `aud` / `exp` / `nbf` validation.
- **`crates/ohd-storage-server/src/auth_server.rs`** — Connect-RPC
  AuthService impl. The five identity RPCs are wired:
  - `ListIdentities` (self-session only, returns full identities including
    subjects).
  - `LinkIdentityStart` (returns `link_token` + `expires_at_ms`).
  - `CompleteIdentityLink` (verifies the supplied id_token against the
    supplied issuer + audiences via the injected JwksResolver).
  - `UnlinkIdentity` (returns `LAST_IDENTITY_PROTECTED` on the last row).
  - `SetPrimaryIdentity` (atomic demote-then-promote).
  The remaining 11 AuthService RPCs (sessions / invites / device tokens /
  notifications / push registration) compile as trait methods that return
  `Unimplemented` — the wire surface is published, the bodies are deferred
  until the deployment system DB lands. See "What's deferred" below.
- **`crates/ohd-storage-server/src/jwks.rs`** — `HttpJwksResolver` with a
  1-hour TTL cache. v1 ships **as a pre-load resolver**: operators call
  `HttpJwksResolver::insert(issuer, JwkSet)` at boot, refresh on a cron.
  The HTTP-fetching path (discovery via
  `<iss>/.well-known/openid-configuration` then `jwks_uri`) is the v1.x
  follow-up — `JwksResolver::resolve` is sync and called from inside the
  storage's `with_conn_mut` closure where the SQLite mutex is held, so
  punting on the network round-trip avoids a tokio-bridge dance that's
  load-bearing for correctness. The seam is one trait, swap-in in `~50` LOC.
- **`proto/ohdc/v0/auth.proto`** — proto evolution:
  - Added `CompleteIdentityLink` RPC + request/response (link_token, id_token,
    issuer, audiences, display_label).
  - Added `SetPrimaryIdentity` RPC + request/response.
  - Extended `Identity` message with `display_label`, `is_primary`,
    `last_login_ms`. (Existing `provider`, `subject`, `email`, `linked_at_ms`
    preserved at their original field numbers.)
  - Extended `LinkIdentityStartResponse` to return `link_token` (renamed
    from `state`) + `expires_at_ms` (new). The `oauth_url` field is
    preserved at its original number for backwards compatibility — server
    leaves it empty in v1, clients build their own URL.
- **`proto/ohdc/v0/ohdc.proto`** — `WhoAmIResponse` now carries
  `repeated LinkedIdentitySummary linked_identities`. Self-session callers
  see their full list (provider + display_label + is_primary + linked_at_ms).
  Grant / device callers see an empty list (a doctor's grant token has no
  business introspecting which OIDC accounts the patient linked). Subjects
  are deliberately not exposed via WhoAmI (no PII leak through grants);
  full identity rows including subjects come via
  `AuthService.ListIdentities` which requires self-session.
- **`build.rs`** — adds `auth.proto` to the proto-compile list. The
  AuthService trait is now codegen'd alongside OhdcService and SyncService.
- **`server::router_with_auth`** — variant of `router()` that takes an
  optional `Arc<dyn JwksResolver>`. When supplied, AuthService is
  registered alongside OhdcService and SyncService on the same Connect-RPC
  router. The default `router()` auto-builds an `HttpJwksResolver`.

### Auth profile rules (multi-identity)

Only **self-session** tokens may manage identities. Grant tokens and
device tokens are rejected at the AuthService boundary with
`WRONG_TOKEN_KIND`. Rationale: a doctor's grant token is for reading /
writing data within a scope, not for changing how the patient authenticates.
Delegate-grant tokens (caregiver acting on behalf of an elderly user) are
also rejected — identity management remains the user's prerogative.

### Tests

- **Unit tests** (`crates/ohd-storage-core/src/identities.rs::tests`) — 7:
  `link_start_creates_pending_row`, `list_returns_empty_for_unknown_user`,
  `bootstrap_then_list`, `unlink_last_identity_refused`,
  `find_user_by_identity_resolves`, `set_primary_promotes_and_demotes`,
  `unlink_primary_promotes_next_oldest`. **All 7 pass** when run against
  ohd-storage-core in isolation (`cargo test -p ohd-storage-core
  identities::`).
- **Integration test — core**
  (`crates/ohd-storage-core/tests/identities_e2e.rs`) — 4 scenarios:
  full-flow with two issuers (link → list → resolve → unlink → last-protected),
  invalid-token rejection (no DB write on verify failure),
  wrong-audience rejection,
  double-complete with same `link_token` rejection,
  cross-user `(iss, sub)` collision returns `IdempotencyConflict`.
  These tests use `RsaPrivateKey` to mint an in-memory JWK set + sign
  RS256 id_tokens against a `StaticJwksResolver`. Dev-deps `rsa = "0.9"`,
  `base64 = "0.22"` are added to the core crate's `[dev-dependencies]`.
- **Integration test — server e2e**
  (`crates/ohd-storage-server/tests/auth_identity_e2e.rs`) — 3 scenarios:
  full link round-trip over Connect-RPC HTTP/2,
  grant-token rejection at AuthService boundary,
  no-bearer rejection.

### JWKS caching strategy

`HttpJwksResolver` keeps a `Mutex<HashMap<String, (Instant, JwkSet)>>` keyed
by issuer URL with a 1-hour TTL (`jwks::JWKS_TTL`). Cache misses on a
stale-or-unknown issuer return `InvalidArgument` directing the operator to
pre-load via `HttpJwksResolver::insert`. The actual HTTP fetch is the
v1.x deliverable — see "Deferred" below.

### Deferred / what's still v1.x

- **HTTP-fetching JWKS resolver**. `HttpJwksResolver::insert` requires the
  operator to push JWK sets at boot + on a refresh cron; the auto-fetch path
  (HTTP GET against `<iss>/.well-known/openid-configuration` then
  `jwks_uri`) is a one-file deliverable that lives on top of the trait.
  Touch only `crates/ohd-storage-server/src/jwks.rs`.
- **`AuthService` other RPCs**. `ListSessions / RevokeSession / Logout /
  LogoutEverywhere / IssueInvite / ListInvites / RevokeInvite /
  IssueDeviceToken / RegisterPushToken / UpdateNotificationPreferences`
  return `Unimplemented`. Bodies land alongside the deployment system DB.
- **Promote-to-primary semantics in invitation flows**. `is_primary` is
  stored, `set_primary` works, `WhoAmI` exposes the flag. The
  *invitation-flow* consumption (e.g. operator invites bind the email_hash
  of the primary identity) is wired in spec but not in any handler yet.
- **`linked_via_actor_id` audit trail**. The column is populated when
  `link_identity_start` knows the session token's rowid; for v1 the
  AuthService handler doesn't have the `_tokens.id` of the bearer (the
  resolved-token surface is rowid-blind in `auth.rs`), so the column
  defaults to NULL. The plumbing-through is a one-line `ResolvedToken`
  extension when the audit need surfaces.

### Coordination notes

- `lib.rs`, `server.rs`, and `Cargo.toml` were touched surgically per the
  cross-agent contract:
  - `lib.rs`: added `pub mod identities;` (concurrent with the
    channel-encryption agent's `pub mod channel_encryption;`).
  - `server.rs`: added `router_with_auth(...)`, extended WhoAmI to populate
    `linked_identities`. The existing handlers and the OhdcAdapter are
    untouched.
  - `Cargo.toml` (workspace): added `jsonwebtoken = "9"` workspace dep.
  - `Cargo.toml` (`ohd-storage-core`): added `jsonwebtoken.workspace =
    true`; dev-deps `rsa = "0.9"`, `base64 = "0.22"`.
  - `Cargo.toml` (`ohd-storage-server`): added `jsonwebtoken.workspace =
    true` (used by `jwks.rs`); dev-deps `rsa`, `base64`, `rand`,
    `jsonwebtoken` for the e2e test's mock issuer.

### Build + test counts after this pass

```
$ cargo build --workspace            # zero warnings
$ cargo test --workspace
... 96 tests pass ...
```

Breakdown:
- 1 conformance corpus run (5 fixtures)
- 38 core unit tests (was 31; +7 `identities::tests`)
- 8 channel encryption e2e (concurrent agent)
- 5 identities e2e (NEW): full link flow / no-DB-write on verification
  failure / wrong-audience rejected / double-complete with same link_token
  rejected / cross-user `(iss, sub)` collision returns
  `IdempotencyConflict`
- 30 pending_grants
- 3 storage smoke
- 3 auth_identity_e2e (NEW): full Connect-RPC link round-trip /
  grant-token rejected at AuthService boundary / no-bearer rejected
- 1 end_to_end Connect-RPC round-trip (existing, includes new
  `linked_identities` field on WhoAmIResponse)
- 4 end_to_end HTTP/3 (existing)

## Sync polish + per-query approval + delegate grants (2026-05-09 evening)

This pass closes out the last two stubbed `SyncService` RPCs and lands the
two remaining design items the previous sweep flagged as deferred (per-query
approval, family/delegate access). Total wired: **35/35 OhdcService + 8/8
SyncService = 43 RPCs**, every one dispatching to a real handler. No stubs
on the wired surface; every `Unimplemented` return is gone.

Plus the delegate issuance helper remains exposed via the in-process API +
tactical helper (the proto frame is read-only canonical for delegate issuance
until v1.x formalizes it on the wire):

- `OhdcService.IssueDelegateGrant` — extends `CreateGrant` with
  `kind=delegate`. Exposed as `ohd_storage_core::ohdc::issue_delegate_grant`.
- `OhdcService.{List,Approve,Reject}PendingQuery` — per-query approval
  queue. **Landed in proto + server dispatch**; still exposed as
  `ohd_storage_core::ohdc::{list,approve,reject}_pending_query` for
  in-process callers. Connect/web's runtime probe should auto-flip from its
  mock path to these real RPCs when pointed at this storage build.

### What landed

- **`SyncService.PushAttachmentBlob`** (client-streaming). Receives chunks,
  validates the supplied sha256 matches the recipient's existing
  `attachments` row (rejects with `NOT_FOUND` if the metadata frame hasn't
  arrived yet). Writes atomically into `<storage_dir>/attachments/<sha[..2]>/<sha>`.
  Idempotent — re-pushing the same payload is a no-op (`outcome="duplicate"`).
- **`SyncService.PullAttachmentBlob`** (server-streaming). Looks up by
  `(ulid, sha256)`, opens the on-disk file, yields `AttachmentInit` →
  64-KiB `Data` chunks → `AttachmentFinish(expected_sha256)`. Auth scope
  matches `ReadAttachment` (self-session; grant tokens are rejected with
  `WRONG_TOKEN_KIND`).
- **Per-peer attachment watermarks**. New table
  `peer_attachment_sync (peer_id, attachment_id, direction)` records
  which attachments crossed which peer in which direction.
  `ohd_storage_core::sync::attachments_pending_delivery` returns the diff
  (rows the peer hasn't seen) so the orchestrator only sends what it
  must. `record_attachment_delivery` and `attachment_delivered` complete
  the watermark surface.
- **`require_approval_per_query`**. New table `pending_queries (ulid,
  grant_id, query_kind, query_hash, query_payload, requested_at_ms,
  expires_at_ms, decision, decided_at_ms, decided_by_actor_id)` plus a
  new `ohd_storage_core::pending_queries` module. When a grant has the
  flag set, the read RPC short-circuits with a new
  `Error::PendingApproval { ulid_crockford, expires_at_ms }` (HTTP 202;
  Connect maps to `FailedPrecondition` with code `PENDING_APPROVAL`).
  Re-issuing the same query returns `PendingApproval` until the user
  approves; on approval the next call passes through. Rejection maps to
  `OUT_OF_SCOPE`; auto-expiry maps to `APPROVAL_TIMEOUT`. Wired into
  `query_events` and `get_event_by_ulid`; the same `check_or_enqueue_approval`
  helper extends to other reads in v1.x.
- **Family / delegate access**. New `grants.delegate_for_user_ulid` column
  + the `grantee_kind="delegate"` invariant. When a delegate-grant token
  resolves, `ResolvedToken.delegate_for_user_ulid` carries the data
  owner's ULID; `is_delegate()` and `effective_user_ulid()` are the
  surface. **Authority is scoped, not unrestricted** — the delegate sees
  only what the grant's per-event-type / per-channel / per-sensitivity
  rules allow. The user can flag specific channels as "self-only" via
  normal `grant_*_rules` to keep them out of the delegate's view.
- **Two-row delegate audit**. New audit column
  `audit_log.delegated_for_user_ulid` + `actor_type='delegate'`. Every
  read RPC under a delegate token writes **two** audit rows via
  `audit::append_for_delegate`: one with `actor_type=Delegate` (the
  caregiver's perspective) and one with `actor_type=Self_` mirror (the
  user's perspective; `grant_id=NULL`, but `delegated_for_user_ulid` set
  so the user sees who acted on their behalf). The user can later
  query `WHERE delegated_for_user_ulid = ?` to get the pair.
- **HTTP/3 progressive streaming verification**. New
  `http3_query_events_progressive_streaming` test seeds 50 events and
  confirms the streaming response arrives in **multiple `recv_data` waves**
  (≥ 2 chunks observed) — proves the `H3RequestBody` ⇄
  `ConnectRpcService` ⇄ h3 `send_data` chain doesn't implicitly buffer.
  This pins the previously-landed P1 work from the 2026-05-08 polish
  pass against accidental regression.

### Migrations added this pass

| File | Purpose |
|---|---|
| `004_peer_attachment_sync.sql` | Per-peer attachment-blob delivery watermark table. |
| `005_pending_queries.sql` | Per-query approval queue table (`require_approval_per_query`). |
| `006_delegate_grants.sql` | Adds `grants.delegate_for_user_ulid` + `audit_log.delegated_for_user_ulid` columns. |

All idempotent; the migration runner ledger gates re-runs. `format.rs`
declares them in order alongside `001`/`002`/`003`.

### Test count (this pass)

```
$ cargo test --workspace
... 55 tests pass ...
```

Breakdown:
- 16 core unit tests (sample_codec round-trip + 5 ulid + …)
- 30 `pending_grants.rs` tests (was 24; +2 sync-attachment, +2 approval-queue, +2 delegate-grants)
- 3 storage-core smoke tests
- 1 end-to-end Connect-RPC round-trip
- 4 end-to-end HTTP/3 tests (was 3; +1 progressive streaming)
- 1 conformance corpus run (5 fixtures)

### What's still deferred (v1.x targets)

- **~~End-to-end channel encryption (E2E for grant data flows)~~** — ✅
  landed in the 2026-05-09 night pass; see "Per-channel end-to-end
  encryption" at the top of this file. The single-storage case ships;
  multi-storage grant re-wrapping is a v0.x follow-up.
- **~~Pending-query proto extension~~** — ✅ landed in the pending-query
  wire pass. `OhdcService.{List,Approve,Reject}PendingQuery` now delegates
  to the existing core helpers, mirroring the existing
  `ListPending`/`ApprovePending`/`RejectPending` pattern.
- **`IssueDelegateGrant` proto extension**. Same shape — core fn ready,
  needs the proto field added to `CreateGrantRequest`
  (`delegate_for_user_ulid`).
- **Pending-query sweep daemon**. `pending_queries::sweep_expired` is
  implemented; needs the same periodic-tokio-task wiring as
  `pending::sweep_expired`.
- **Encryption-at-rest hierarchy** (`K_file` / `K_envelope` / `K_recovery`).
  SQLCipher key works against the raw 32 bytes. The three-layer
  hierarchy from `spec/encryption.md` is still v1.x.
- **`AuthService` / `RelayService` Rust handlers**. The proto exists;
  Auth is the OIDC dance owned by the consumer apps, Relay is owned
  by the relay-hardening agent.
- **Typed `ErrorDetail` carrier**. Errors today encode the OHDC code
  via the `"OHDC_CODE: text"` message prefix; a follow-up pass packs a
  typed `ErrorDetail` (proto3 `ErrorInfo`) so generic Connect / gRPC
  clients can read it as structured data.
- **Sync orchestrator daemon**. The wire surface is complete; what's
  missing is the periodic foreground loop + push-wake hook that drives
  Hello → PushFrames → PullFrames → Push/PullAttachmentBlob on a
  schedule. Today that loop lives in the consumer apps.
- **Selective sync filters** (per-event-type / per-time-range exclusions
  on caches), **multi-primary topologies** with conflict resolution,
  **per-platform binding packaging** (`.aar` / `.xcframework` / wheel)
  remain explicitly v1.x.

### RPC count after this pass

| Service | Total | Wired | Stubbed |
|---|---|---|---|
| OhdcService | 28 | 28 | 0 |
| SyncService | 8 | 8 | 0 |
| AuthService | 0 (proto compiled but no handlers; see "deferred") | — | — |
| RelayService | 0 | — | — |

Plus 1 proto-pending RPC exposed via a core helper (`IssueDelegateGrant`);
the pending-query trio is now wired over Connect-RPC.

## v1 RPC sweep (2026-05-09 morning)

The remaining seven `OhdcService` RPCs (`AttachBlob`, `Aggregate`, `Correlate`,
`ReadSamples`, `ReadAttachment`, `AuditQuery`, `Export`, `Import`) plus all
nine `Case*` RPCs are now wired end-to-end. Total wired: **28/28
OhdcService + 8 SyncService = 36 RPCs**. Of those, 36 dispatch to real
handlers; the only deliberate stubs left are `PushAttachmentBlob` and
`PullAttachmentBlob` on `SyncService` (the ohd-storage-core attachment
plumbing covers the same blob store; sync-side delivery is a polish-pass
deliverable).

### What landed

- **Sample-block codecs** (`crates/ohd-storage-core/src/sample_codec.rs`).
  Encoding 1 (delta-zigzag-varint timestamps + float32 values, zstd level 3)
  and Encoding 2 (delta-zigzag-varint + int16 quantized + scale, zstd level
  3). Determinism is conformance-load-bearing and asserted by 11 unit tests
  + 3 corpus fixtures.
- **`ReadSamples`** decodes every block on `(event_ulid, channel_path)`,
  honours optional `[from_ms, to_ms]` slice + `max_samples` downsample, and
  streams `Sample` batches of 1024 over Connect-RPC.
- **`Aggregate`** buckets by fixed `Duration` or calendar unit
  (`HOUR`/`DAY`/`WEEK`/`MONTH`/`YEAR`), supports `AVG`/`SUM`/`MIN`/`MAX`/
  `COUNT`/`MEDIAN`/`P95`/`P99`/`STDDEV`. The aggregation_only grant flag
  unblocks this RPC (the only read RPC such grants can call).
- **`Correlate`** finds for each `a`-side event the `b`-side events whose
  timestamp falls within a symmetric `±window/2` slice. Returns
  `CorrelatePair` per `a`-side row with the matched `b`-side ULIDs +
  values, plus aggregate `CorrelateStats` (paired_count, mean_b_value,
  mean_lag_ms). Pearson correlation as a derived stat is left for v1.x —
  the wire shape is the load-bearing contract.
- **`AttachBlob`** (client-streaming): drains chunks into a temp file,
  computes sha256, atomically renames into
  `<storage_dir>/attachments/<sha[..2]>/<sha>` + inserts `attachments` row.
  50 MiB cap (configurable per-call). Sidecar files are unencrypted — the
  encryption-at-rest hierarchy is a v1.x deliverable; SQLCipher protects
  only `data.db`.
- **`ReadAttachment`** (server-streaming): looks up by ULID, opens the
  on-disk file, yields `AttachmentInit` → 64-KiB `Data` chunks →
  `AttachmentFinish(expected_sha256)`.
- **`AuditQuery`** (server-streaming): self-session sees all rows; grant
  tokens are scoped to their own (we override `grant_id` in the query).
- **`Export` / `Import`** (server-streaming / client-streaming): produces an
  `Init { format_version, source_instance_pubkey_hex } || Event* || Grant*
  || AuditEntry* || Finish` frame stream; import is idempotent on event
  ULIDs (existing rows skipped silently, returns `events_imported=0`).
  Encryption + Ed25519 signing of exports are placeholders today (left for
  v1.x alongside the encryption hierarchy).
- **All `Case*` RPCs** (CreateCase, UpdateCase, CloseCase, ReopenCase,
  ListCases, GetCase, AddCaseFilter, RemoveCaseFilter, ListCaseFilters)
  delegate to the existing `ohd_storage_core::cases` machinery — these
  weren't part of the seven stubs but were also stubbed in the previous
  pass; cleanup landed in this sweep.

### Full grant resolver (P3 from the previous status)

`crates/ohd-storage-core/src/events.rs::GrantScope` now carries the full
precedence ladder per `spec/storage-format.md` "Combination precedence":

1. Sensitivity-class deny (event-type level + channel level)
2. Channel deny (per `grant_channel_rules`)
3. Event-type deny
4. Sensitivity-class allow
5. Channel allow
6. Event-type allow
7. `grants.default_action` (fallback)

Plus:

- **Time windows** — `rolling_window_days` and absolute `(from_ms, to_ms)`
  are honoured at the row level. Out-of-window events count toward
  `rows_filtered`.
- **Per-channel filtering** — `grant_channel_rules` rows now influence which
  channels survive on a returned event. The grantee never sees stripped
  channels (silent strip; appears in `rows_filtered` only when the entire
  event is dropped, not when a sub-channel is hidden).
- **Rate limits** — `max_queries_per_day` / `max_queries_per_hour` are
  enforced at scope-materialization time. Counts come from `audit_log`
  rows for the grant within the trailing window. Excess returns
  `RATE_LIMITED`.
- **Channel rules from the wire** — `CreateGrantRequest.channel_rules`
  decode by splitting the dotted path into `(event_type, channel_path)`
  using `std.<name>.<path>` / `com.<owner>.<name>.<path>` heuristics.
  Malformed paths drop silently rather than failing the create.

`require_approval_per_query` is **stored** in the schema (via `CreateGrant`)
but **not** enforced — the per-query approval queue is documented in
`spec/privacy-access.md` as needing UI plumbing through Connect; v1.x adds
the approval-queue table + the timeout daemon.

### Sync (P7)

`SyncService` is registered alongside `OhdcService` on the same router.

- **Hello** discovery upserts the peer row (`peer_sync` table) and replies
  with our local high-water rowid + the inbound watermark we have for that
  peer + the user's ULID for first-contact pairing.
- **PushFrames** drains the inbound stream, applies events idempotently
  (ULID dedup; `apply_inbound_event` returns `false` for existing ULIDs),
  advances the per-peer inbound watermark, and acks each frame as `ok` /
  `duplicate` / `rejected`.
- **PullFrames** returns events with `events.id > after_peer_rowid` while
  skipping rows whose `origin_peer_id` matches the requesting peer (echo
  suppression). Bounded by `max_frames` (default 1000).
- **CreateGrantOnPrimary / RevokeGrantOnPrimary / UpdateGrantOnPrimary**
  delegate to the corresponding `OhdcService` handlers under the user's
  self-session token.
- **PushAttachmentBlob / PullAttachmentBlob** are deliberate stubs —
  consumer-side `OhdcService.AttachBlob` writes the same blob store, so
  cache↔primary attachment delivery just needs a per-peer trigger that
  v1.x can land alongside the sync orchestrator.

Auth between peers: must be a self-session token. Grant tokens are rejected
with `WRONG_TOKEN_KIND` per `spec/sync-protocol.md` "Auth between peers".

### Conformance harness (P8)

New crate: `crates/ohd-storage-conformance/`. The corpus lives at
`corpus/manifest.json` with these v1 fixtures:

| Path | Required | Asserts |
|---|---|---|
| `sample_blocks/encoding1/001_simple` | yes | Encoding-1 byte-determinism on 4 samples |
| `sample_blocks/encoding1/002_dense` | yes | Encoding-1 byte-determinism on 60 samples |
| `sample_blocks/encoding2/001_simple` | no  | Encoding-2 byte-determinism on 4 samples (encoding 2 is "strongly recommended" per spec, not required) |
| `ohdc/put_query/001_self_session_round_trip` | yes | Self-session put → query round-trip with two events of different types |
| `permissions/001_event_type_deny` | yes | `default_action='allow' + event_type_rule(deny)` correctly silently drops the denied type |

Runner usage:

```sh
# Run all fixtures.
cargo test -p ohd-storage-conformance corpus_passes

# Regenerate the sample-block expected.bin files (after editing input.json).
cargo run -p ohd-storage-conformance --bin regen
```

Categories `sync/*`, `auth/*`, `streaming/*` and `pagination/*` are
deferred to v1.x — the corpus directory tree exists in
`spec/conformance.md` but the v1 reference impl ships the seed above only.
Documented in `crates/ohd-storage-conformance/src/lib.rs` "How to add a
fixture".

### RPC count

| Service | Total | Wired | Stubbed |
|---|---|---|---|
| OhdcService | 28 | 28 | 0 |
| SyncService | 8 | 6 | 2 (PushAttachmentBlob, PullAttachmentBlob — see above) |
| AuthService | 0 (proto compiled but no handlers; see "What's stubbed") | — | — |
| RelayService | 0 | — | — |

### Test count (this pass)

```
$ cargo test --workspace
... 48 tests pass ...
```

Breakdown:

- 16 core unit tests (sample_codec round-trip + 5 ulid + …)
- 24 `pending_grants.rs` tests (grant CRUD + new RPCs + resolver + sync apply)
- 3 storage-core smoke tests
- 1 end-to-end Connect-RPC round-trip
- 3 end-to-end HTTP/3 tests
- 1 conformance corpus run (5 fixtures)



## Wire-format swap (2026-05) — Connect-RPC over HTTP/2

The first pass shipped JSON-over-HTTP/1.1 because the implementing agent
believed Rust ConnectRPC tooling wasn't ready. **That assumption was wrong**;
the v1 server now speaks **real Connect-RPC framing**: binary Protobuf bodies
(buffa-encoded), Connect-Protocol headers / gRPC trailers, codegen-emitted
service traits and clients. JSON is still negotiable per request — it's a
codec choice on the same handlers, not a separate transport.

Pinned versions and rationale:

| Crate | Version | Why |
|---|---|---|
| `connectrpc` | `0.4` (latest at the time, 0.4.2) | Tower-based ConnectRPC runtime maintained by Anthropic. Speaks Connect, gRPC, gRPC-Web on the same handlers. |
| `connectrpc-build` | `0.4` | `build.rs` codegen integration. |
| `buffa` | `0.5` (with `json` feature) | Pure-Rust Protobuf with first-class `View` (zero-copy decode) types. |
| `buffa-types` | `0.5` (with `json`) | Companion to buffa for proto3 JSON helpers. |
| `protoc-bin-vendored` | `3` (build-dep) | Ships a bundled `protoc` so consumers don't need a system install. |
| `quinn` | — (not yet) | HTTP/3 transport. **Not currently wired** (see "HTTP/3 deferred" below). |
| `hyper` / `hyper-util` / `http-body` / `tower` | as required by connectrpc | Underlying HTTP/1.1 + HTTP/2 stack. |

The MSRV moved from 1.83 → 1.88 (workspace `rust-version`). `connectrpc 0.4`
declares 1.88 because its generated code targets edition-2024 features.
System rustc on the dev box is 1.94.

### HTTP/3 polish pass (2026-05-08, follow-ups landed)

Three follow-ups landed on top of the original in-binary HTTP/3 listener:

- **Production cert flags (P0)**. `serve` accepts `--http3-cert PATH` /
  `--http3-key PATH` to load a PEM-encoded cert chain + private key from
  disk via `rustls-pemfile 2`. `http3::load_pem_cert_key` is the loader;
  it accepts PKCS#8, PKCS#1, and SEC1 keys, errors out cleanly on missing
  files / empty PEM blocks, and is exercised by the new
  `http3_load_pem_cert_key` integration test (round-trips an
  `rcgen`-minted PEM pair through the loader and brings up a server with
  it). When neither flag is set the listener falls back to
  `dev_self_signed_cert()` and emits a stderr warning so production
  misconfiguration is hard to miss.
- **Streaming `http_body::Body` for HTTP/3 (P1)**. The previous H3 path
  buffered the entire request body to a single `Bytes` (4 MiB cap). The
  new `http3::H3RequestBody` implements `Body<Data = Bytes>` and pulls
  chunks from `RequestStream::recv_data` lazily inside `poll_frame`, so
  server-streaming RPCs (`QueryEvents`, future `AttachBlob` / `Import`)
  no longer need to fit in the buffer. The response writer is unchanged
  (per-frame `send_data` was already streaming). The new
  `http3_query_events_streaming` test seeds one event, dials the server
  over h3, and round-trips a Connect-protocol streaming `QueryEvents`
  request — validates the body adapter end-to-end and pins the
  `application/connect+proto` response content-type.
- **Connect CLI HTTP/3 client variant (P2)**. New `H3RawClient` in
  `connect/cli/src/client.rs` talks directly to a server's QUIC port,
  bypassing connectrpc on the client side (connectrpc 0.4 ships only
  `Http2Connection`). The CLI now accepts `https+h3://host:port` URLs and
  an `--insecure-skip-verify` flag for dev / self-signed certs. See
  `connect/STATUS.md` "HTTP/3 client" for the full detail.

### HTTP/3 (in-binary) — landed (2026-05-08)

`connectrpc 0.4.2` doesn't ship a quinn integration, but it doesn't need
one: `connectrpc::ConnectRpcService` is a transport-agnostic
`tower::Service<http::Request<B>>`, and any HTTP/3 framing crate that hands
us an `http::Request` can drive it. We wired that adapter directly:

- New module `crates/ohd-storage-server/src/http3.rs` runs an in-binary
  HTTP/3 listener on top of `quinn 0.11` + `h3 0.0.8` + `h3-quinn 0.0.10`.
  Per QUIC connection: `h3::server::Connection::new()` accepts request
  streams; per request: we drain the body via `RequestStream::recv_data` to
  a single `Bytes`, build an `http::Request<Full<Bytes>>`, clone the shared
  `ConnectRpcService`, and pump the response body's frames back through
  `send_data` / `send_trailers` / `finish`. ~250 LOC of glue.
- `--http3-listen 0.0.0.0:18443` flag on `serve` opts the listener in.
  Both listeners run concurrently on the same `Storage` handle so handler
  bodies are identical regardless of transport. Default off so existing
  `serve --listen` behaviour is unchanged.
- Self-signed cert helper (`http3::dev_self_signed_cert`) uses `rcgen
  0.13` — valid for `localhost` / `127.0.0.1`, suitable for dev + the
  integration test. Production cert flags (`--http3-cert PATH` /
  `--http3-key PATH`) are a follow-up; the helper code is ready for a
  loader to call.
- `rustls 0.23` pinned with `default-features = false` + `ring` (no
  `aws-lc-rs` → no aws-lc-sys C build). ALPN is set to `b"h3"` on a
  hand-built `rustls::ServerConfig` (TLS 1.3 only,
  `max_early_data_size = u32::MAX`) so QUIC's ALPN negotiation succeeds.

#### Why in-binary, not Caddy-fronted

We deliberately did not ship "Caddy speaks HTTP/3, storage stays HTTP/2"
for three reasons:

1. **Single static binary** is in the spec. Caddy adds another moving part
   operators have to install + monitor. "Deploy in an afternoon" stops
   being honest.
2. **Socket count under high event load** — large sensor / EHR fleets push
   continuous device-token writes (Libre, Dexcom, Garmin, lab providers,
   …); QUIC's connection migration + cheap stream suspend/resume scales
   better than holding thousands of TCP sockets.
3. **Mobile clients** (Connect on iOS/Android over flaky cellular) are the
   primary OHDC client. QUIC's handshake-resume + connection-migration
   beat TCP for that workload.

Caddy-fronted HTTP/3 is still a valid deployment topology — operators who
already run Caddy can keep doing so and skip `--http3-listen`. The
in-binary path adds a deployment option, doesn't remove one.

#### Trailers — sidestepped via Connect protocol

`h3 0.0.8` does support trailers (`RequestStream::send_trailers`), but
trailer interop in the broader HTTP/3 ecosystem is uneven. The HTTP/3 path
defaults to **Connect protocol** content-types
(`application/proto` for unary, `application/connect+proto` for streaming
— see `connectrpc::protocol::Protocol::detect_from_content_type`), where
the trailing status lives in a final body envelope, **not** in HTTP
trailers. The HTTP/2 path keeps `application/grpc+proto` (trailer-based)
unchanged for clients that prefer gRPC. The integration test pins
`application/proto` on the response so a future regression to
JSON-over-HTTP/3 or gRPC-trailer-over-HTTP/3 fails the test.

#### Pinned versions

| Crate | Version | Notes |
|---|---|---|
| `quinn` | `0.11` (resolved 0.11.9) | rustls 0.23 + ring crypto provider, runtime-tokio. |
| `h3` | `0.0` (resolved 0.0.8) | server + client trait surface. |
| `h3-quinn` | `0.0` (resolved 0.0.10) | Connection / BidiStream / RecvStream adapters. |
| `rustls` | `0.23` (resolved 0.23.40) | `default-features = false`, `ring` + `std` + `tls12` features. |
| `rcgen` | `0.13` (resolved 0.13.2) | `ring` feature; ECDSA P-256 self-signed cert. |

## Pending + Grant wire RPCs landed (2026-05-08)

The pending-flow trio (`ListPending`, `ApprovePending`, `RejectPending`) and
the grant CRUD quad (`CreateGrant`, `ListGrants`, `UpdateGrant`,
`RevokeGrant`) are now wired wire RPCs over Connect-RPC, replacing the CLI
hacks. The CLI subcommands (`issue-grant-token`, `pending-list`,
`pending-approve`) print a `DEPRECATED:` banner and remain as ops shortcuts;
they will be deleted in v1.x.

The `EventFilter` language also expanded: `device_id_in`, `source_in`,
`event_ulids_in`, `sensitivity_classes_in/_not_in`, and
`channel_predicates` (`eq` / `neq` / `gt` / `gte` / `lt` / `lte`, AND-of)
are honoured. Channel predicates run as a post-query filter pass over the
events that match the cheap predicates — the perf trade-off is documented
in the `query_events` doc comment; SQL-native channel-predicate evaluation
is a v1.x optimization once we have the conformance corpus to anchor it.

The grant resolution algorithm now applies sensitivity-class deny at the
event-type level (using `event_types.default_sensitivity_class`) and honours
`strip_notes` on returned rows. Per-channel grant rules, `aggregation_only`,
`require_approval_per_query`, time windows, and rate limits are all stored
correctly via `CreateGrant` (the schema has the rows) but the resolver only
actively enforces `aggregation_only` (blocks `query_events`) and
`strip_notes`. Full P3 resolution lands in v1.x once the conformance corpus
is executable.

End-to-end `connect_rpc_round_trip` test now also drives:
- doctor (`grant` token) `PutEvents` → returns `pending`,
- patient (`self_session`) `ListPending` → sees the doctor's submission,
- patient `ApprovePending` → ULID preserved across promotion,
- patient `ListGrants` → sees ≥ 1 grant.

## OHD Care wiring pass (2026-05-08)

The `ohd-storage-server` binary picked up three demo-driving subcommands and a
permissive CORS layer so the OHD Care web app at `http://localhost:5173` can
reach the storage process at `http://localhost:18443` directly during dev.

| Subcommand | What it does |
|---|---|
| `issue-grant-token --db PATH --read CSV --write CSV --approval-mode {…} --label STR --expires-days N` | **DEPRECATED** — superseded by `OhdcService.CreateGrant`. Still creates a `grants` row + bearer token (`ohdg_…`) in one shot for ops use. |
| `pending-list --db PATH` | **DEPRECATED** — superseded by `OhdcService.ListPending`. Still dumps `pending_events` rows directly from SQLite for ops use. |
| `pending-approve --db PATH --ulid X` | **DEPRECATED** — superseded by `OhdcService.ApprovePending`. Still promotes a `pending_events` row into `events` + `event_channels`, preserving the ULID, for ops use. |

`serve` accepts a new `--no-cors` flag (default off). When CORS is on, the
router is wrapped in `tower_http::cors::CorsLayer::very_permissive()` with
gRPC-Web trailers exposed (`grpc-status`, `grpc-message`, `grpc-status-details-bin`)
so browser fetch + Connect-Web works against a different origin.

The `issue-grant-token` subcommand also seeds `std.clinical_note` (with `text` +
`author` channels) into the registry on first use, since it isn't yet in
`002_std_registry.sql`. This is `INSERT OR IGNORE` — re-running is a no-op
once the type exists. Pickup: add `migrations/003_clinical_note.sql` once the
storage v1.x pass goes there, and drop the helper.

The `pending-approve` path is implemented by the storage server crate (not
ohd-storage-core) using a direct rusqlite transaction — promote → audit →
commit. When `OhdcService.ApprovePending` lands, that handler should mirror
the same precedence (preserve ULID, audit `pending_approve`).

The Care web app's end-to-end demo lives at `care/demo/run.sh` (driver) +
`care/demo/README.md` (walkthrough).

## Bindings (2026-05-08)

`crates/ohd-storage-bindings` graduated from an `rlib` re-export to a real
foreign-language facade. uniffi 0.28 in proc-macro mode (no `.udl`); the
namespace is `ohd_storage` (Kotlin: `package uniffi.ohd_storage`; Swift:
`enum OhdStorage`; Python: `import ohd_storage`). The crate type list is
`["cdylib", "staticlib", "rlib"]` so the same source compiles for Android
`.so`, iOS `.xcframework` (static archive), and Rust integration tests.

### Surface exposed

`crates/ohd-storage-bindings/src/lib.rs` exposes:

| Item | Kind | Notes |
|---|---|---|
| `OhdStorage` | uniffi `Object` (Arc) | Thread-safe handle to one open per-user storage file. |
| `OhdStorage.open(path, key_hex)` | constructor | Open existing file. Errors if file missing. |
| `OhdStorage.create(path, key_hex)` | constructor | Create-or-open + run migrations. |
| `OhdStorage.path()` | method | Backing file path. |
| `OhdStorage.user_ulid()` | method | Crockford-base32 ULID stamped at create. |
| `OhdStorage.put_event(EventInputDto)` | method | One-event wrapper around `events::put_events`. |
| `OhdStorage.query_events(EventFilterDto)` | method | Self-session-scoped event query. |
| `OhdStorage.issue_self_session_token()` | method | Mint `ohds_…`; cleartext returned exactly once. |
| `OhdStorage.format_version()` / `protocol_version()` | method | Version surface. |
| `storage_version()`, `protocol_version()`, `format_version()` | top-level fn | Version probes (no handle needed). |
| `OhdError` | uniffi `Error` enum | Five variants: `OpenFailed`, `Auth`, `InvalidInput`, `NotFound`, `Internal` — collapsed from `core::Error`'s 25+ internal variants. OHDC error code preserved in `code` for audit. |
| `EventInputDto` / `EventFilterDto` / `EventDto` / `PutEventOutcomeDto` / `ChannelValueDto` | uniffi `Record` | DTOs crossing the FFI boundary. |
| `ValueKind` | uniffi `Enum` | Discriminant for `ChannelValueDto` (`REAL` / `INT` / `BOOL` / `TEXT` / `ENUM_ORDINAL`). |

The DTOs flatten core types that uniffi 0.28 can't carry losslessly across
FFI (`Mutex<Connection>`, lifetime-bound errors, untagged enums). The
`ChannelValueDto` is a tagged-record-with-discriminant pattern: exactly
one of `real_value` / `int_value` / `bool_value` / `text_value` /
`enum_ordinal` is set per `value_kind`. Kotlin / Swift / Python call sites
look like:

```kotlin
ChannelValueDto(
    channelPath = "value",
    valueKind = ValueKind.REAL,
    realValue = 6.4,
    intValue = null, boolValue = null, textValue = null, enumOrdinal = null,
)
```

### How to build the per-platform artifacts

The recipes are documented in:

- **Android `.aar` / `.so`**: `connect/android/BUILD.md` — three stages
  (`cargo ndk` per ABI → `uniffi-bindgen --language kotlin` → `gradle
  assembleRelease`).
- **iOS `.xcframework`**: `connect/ios/BUILD.md` (TBD; the Cargo.toml
  already declares `staticlib` so the iOS recipe is a small extension of
  the Android one — `cargo build --target aarch64-apple-ios{,-sim}`,
  `xcodebuild -create-xcframework`, `uniffi-bindgen --language swift`).
- **Python wheel**: `cargo run -p ohd-storage-bindings --features cli --bin
  uniffi-bindgen -- generate --library target/release/libohd_storage_bindings.so
  --language python --out-dir <pkg>/`. Wheel packaging via maturin or
  setuptools-rust is a separate v1.x deliverable; the `pyo3` direct
  binding mentioned in the early roadmap (#11 below) is now redundant
  because uniffi covers Python too.

### Workspace build status

`cargo build --workspace` compiles `ohd-storage-bindings` cleanly under
proc-macro mode (uniffi 0.28's `setup_scaffolding!("ohd_storage")`
expands at compile time; the `cdylib` + `staticlib` + `rlib` triple ships
on every host). The default feature set (`default = []`) keeps the
bindgen CLI dependency tree off the regular build path; `--features cli`
is required only to run the standalone `uniffi-bindgen` binary.

Connect Android consumes this surface in
`connect/android/app/src/main/java/com/ohd/connect/data/StorageRepository.kt`,
behind TODO-marked stubs that swap one-for-one with
`uniffi.ohd_storage.OhdStorage` once `connect/android/BUILD.md` Stage 1
+ Stage 2 have run on a developer machine with the NDK installed.

## What's done (v1 — first useful pass)

### Build, run, test

```
cargo build        # clean, zero warnings
cargo test         # 6 tests pass:
                   #   - ohd-storage-core::ulid::tests::pre_1970_clamp
                   #   - ohd-storage-core::ulid::tests::roundtrip_encode_decode
                   #   - ohd-storage-core::smoke::smoke_self_session_round_trip
                   #   - ohd-storage-core::smoke::smoke_alias_resolution
                   #   - ohd-storage-core::smoke::smoke_token_kind_matrix
                   #   - ohd-storage-server::end_to_end::connect_rpc_round_trip
cargo run -p ohd-storage-server -- health
                   # → OHD Storage server v0.0.0 — health: ok (protocol ohdc.v0)
cargo run -p ohd-storage-server -- init --db ./data.db
                   # → initialized storage at ./data.db (user_ulid=...)
cargo run -p ohd-storage-server -- issue-self-token --db ./data.db
                   # → ohds_<base32>
cargo run -p ohd-storage-server -- serve --db ./data.db --listen 0.0.0.0:8443
                   # → starts a Connect-RPC server (HTTP/1.1 + HTTP/2; HTTP/3 next)
```

**Test count (post pending+grant wire pass):** 22 tests pass workspace-wide
— 2 ulid unit, 3 storage-core smoke, 13 `pending_grants.rs` unit, 1
end-to-end Connect-RPC, 3 end-to-end HTTP/3.


End-to-end **Connect-RPC** round-trip lives in
`crates/ohd-storage-server/tests/end_to_end.rs` and asserts:

  - `OhdcService.Health` (unauthenticated) returns
    `{ status: "ok", protocol_version: "ohdc.v0", … }` and the response carries
    `content-type: application/grpc+proto` (binary framing — _not_ JSON).
  - `OhdcService.WhoAmI` returns the resolved actor info under a
    self-session bearer; without one, it returns `ErrorCode::Unauthenticated`.
  - `OhdcService.PutEvents` returns `PutEventResult::committed` with a
    16-byte wire ULID.
  - `OhdcService.QueryEvents` (server-streaming) yields the matching event
    back over the stream and matches its ULID byte-for-byte.
  - `OhdcService.GetEventByUlid` returns the same event with the same ULID.

The test drives the codegen-emitted `OhdcServiceClient<Http2Connection>`
over plaintext HTTP/2 (`Http2Connection::connect_plaintext`) using the
`Protocol::Grpc` codec, so the wire is exactly what production speaks.

### Schema and registry seed

- `migrations/001_initial_schema.sql` lays down every spec table:
  `_meta`, `event_types`, `channels`, `type_aliases`, `channel_aliases`,
  `devices`, `app_versions`, `peer_sync`, `events`, `event_channels`,
  `event_samples`, `attachments`, `cases`, `case_filters`,
  `case_reopen_tokens`, `grants`, `grant_cases`, `grant_event_type_rules`,
  `grant_channel_rules`, `grant_sensitivity_rules`,
  `grant_write_event_type_rules`, `grant_auto_approve_event_types`,
  `grant_time_windows`, `pending_events`, `audit_log`, plus the new `_tokens`
  table for the per-file token store. All `IF NOT EXISTS` / `INSERT OR IGNORE`,
  applied via a tiny ledger in `_meta` (`mig:001_initial_schema = …`).
- `migrations/002_std_registry.sql` seeds the standard registry: `std.blood_glucose`
  (with `std.glucose` alias), `std.heart_rate_resting` (+ `std.heart_rate`
  alias), `std.heart_rate_series`, `std.body_temperature` (+ `std.temperature`
  alias), `std.blood_pressure`, `std.medication_dose` (+ `std.medication_taken`
  alias), `std.symptom`, `std.meal` (+ `std.food` alias) with a partial
  nutrition tree, and `std.mood` for the sensitivity-class fixture.
- The runner is in `crates/ohd-storage-core/src/format.rs` —
  embeds migrations via `include_str!`, applies missing steps once, gates
  on `_meta.mig:<name>`, verifies `_meta.format_version` matches the build.

### Per-user file open + encryption-at-rest

- `Storage::open(StorageConfig)` opens or creates a per-user `data.db`,
  applies SQLCipher `PRAGMA key = "x'<hex>'"` when a `cipher_key` is supplied
  (`bundled-sqlcipher` feature on `rusqlite`), enables WAL mode, and runs the
  migration ledger.
- `_meta.user_ulid`, `deployment_mode`, `created_at_ms`, `format_version`,
  `registry_version` are stamped on creation.
- `encryption::KeyProvider` trait + `StaticKeyProvider` for the in-memory
  testing case. Production keystore plumbing is the v1.x deliverable.

### Auth profile resolution

- Token classification by prefix (`ohds_` / `ohdg_` / `ohdd_`) → kind.
- Tokens are stored as `sha256(bearer)` in the `_tokens` table colocated in
  the per-user file (deviation from spec — see "Decisions" below).
- `auth::issue_self_session_token`, `auth::issue_grant_token`,
  `auth::resolve_token`, `auth::check_kind_for_op`.
- Token-kind matrix enforced (self-session = full; grant = read/write events;
  device = write only). Wrong-kind ops return `WRONG_TOKEN_KIND`. Templates
  (`grants.is_template=1`) are rejected at resolve time per spec.

### OHDC server (Health, WhoAmI, PutEvents, QueryEvents, GetEventByUlid + pending trio + grant CRUD)

- `crates/ohd-storage-server` exposes the five RPCs via **Connect-RPC over
  HTTP/2** (`connectrpc::Server` on `hyper-util`, with the codegen-emitted
  `OhdcService` trait). The server speaks Connect, gRPC, and gRPC-Web on
  the same handlers; codec (Proto / JSON) is per-request. The full proto
  schema (`proto/ohdc/v0/ohdc.proto`) is compiled at build time by
  `connectrpc-build`. The transport adapter
  ([`server::OhdcAdapter`](crates/ohd-storage-server/src/server.rs)):
  1. Resolves the bearer token from `Authorization: Bearer …` headers
     (`auth::resolve_token`) — `ErrorCode::Unauthenticated` on miss.
  2. Materializes the `OwnedView<…RequestView>` into the owned message
     (`view.to_owned_message()`) so the existing in-process core API
     (which takes plain Rust types) stays unchanged.
  3. Calls into `ohd_storage_core::ohdc::*`.
  4. Translates `Error` into a `ConnectError` whose code is the
     Connect / gRPC status that matches `Error::http_status()`, with the
     OHDC error code prefixed into the message (`"OUT_OF_SCOPE: ..."` etc.)
     pending a typed `ErrorDetail` carrier (see below).
- The pending-flow trio (`ListPending`, `ApprovePending`, `RejectPending`)
  and grant CRUD quad (`CreateGrant`, `ListGrants`, `UpdateGrant`,
  `RevokeGrant`) are also wired against `ohd_storage_core::ohdc::*`, with
  the same auth + audit pattern as the original five. **Total wired: 12**.
- The remaining ~17 RPCs in the proto (`Aggregate`, `Correlate`,
  `AttachBlob`, `ReadSamples`, `ReadAttachment`, `AuditQuery`, `Export`,
  `Import`, all `Case*` ops) compile but return `ErrorCode::Unimplemented`;
  their handler bodies land alongside the rest of the v1.x deliverables.
- `PutEvents` validates each event against the registry (event-type lookup
  including aliases, per-channel value-type and enum-ordinal checks),
  enforces idempotency on `(source, source_id)`, allocates a ULID per event,
  routes through `pending_events` when the grant has `approval_mode='always'`,
  otherwise commits to `events` + `event_channels`. Returns one
  `PutEventResult` per input (`committed` / `pending` / `error`).
- `QueryEvents` filters by time range + event-type allow/deny, applies the
  grant-scope event-type allow/deny intersection (deny > allow > default),
  and reports `rows_filtered` distinct from `rows_returned`.
- `GetEventByUlid` parses the Crockford-base32 input, looks up by
  `ulid_random`, applies grant scope (returns `NOT_FOUND` for out-of-scope
  rows per the no-leak rule).
- Every operation appends one `audit_log` row, including for rejections.
  Audit captures `actor_type`, `grant_id`, `action`, `query_kind`,
  `query_params_json` (canonicalized), `rows_returned`, `rows_filtered`,
  `result`, `reason`.

### Smoke tests

- `crates/ohd-storage-core/tests/smoke.rs`:
  - `smoke_self_session_round_trip` — open temp DB → mint self-session token →
    PutEvents one `std.blood_glucose` → QueryEvents → GetEventByUlid →
    confirm `audit_log` has ≥ 3 rows. Uses a 32-byte cipher key.
  - `smoke_alias_resolution` — `std.glucose` → `std.blood_glucose` via
    `type_aliases`.
  - `smoke_token_kind_matrix` — bad tokens return `Unauthenticated`.
- `crates/ohd-storage-server/tests/end_to_end.rs`:
  - `connect_rpc_round_trip` — boots `connectrpc::Server` on an ephemeral
    port, drives the codegen `OhdcServiceClient` over plaintext HTTP/2
    (`Http2Connection::connect_plaintext`) with `Protocol::Grpc`, and
    asserts the put/query/get round-trip plus
    `ErrorCode::Unauthenticated` on missing bearer. Also pins the
    response `content-type` to `application/grpc+proto` so a future
    accidental fall-back to JSON would fail the test.

## Tree of changes since the wire-format swap

Files touched in the Connect-RPC swap (everything else is unchanged from the
previous pass):

```
storage/
├── Cargo.toml                          (UPDATED — added connectrpc / buffa /
│                                         hyper / http-body / bytes / futures
│                                         workspace deps; bumped MSRV to 1.88)
├── STATUS.md                           (UPDATED — this file)
└── crates/
    └── ohd-storage-server/
        ├── Cargo.toml                  (UPDATED — wire deps + connectrpc-build /
        │                                 protoc-bin-vendored as build-deps;
        │                                 dropped reqwest dev-dep)
        ├── build.rs                    (NEW — invokes connectrpc-build with
        │                                 the vendored protoc; emits
        │                                 $OUT_DIR/_connectrpc.rs)
        ├── src/
        │   ├── main.rs                 (UPDATED — exposes `proto` module via
        │   │                             `connectrpc::include_generated!()`;
        │   │                             default `--listen` now `0.0.0.0:8443`)
        │   └── server.rs               (REWRITTEN — `OhdcAdapter` impls the
        │                                 codegen `OhdcService` trait; five
        │                                 wired RPCs dispatch to
        │                                 `ohd_storage_core::ohdc::*`; the
        │                                 other 25 stub `Unimplemented`)
        └── tests/
            └── end_to_end.rs           (REWRITTEN — uses the codegen
                                          OhdcServiceClient over HTTP/2 with
                                          gRPC framing; pins content-type)
```

`crates/ohd-storage-core/*` was **not** modified during the swap — the
business-logic functions (`ohd_ohdc::health`, `whoami`, `put_events`,
`query_events`, `get_event_by_ulid`) keep their existing signatures.

## What's stubbed / deferred (next-phase pickup)

The following are part of the canonical spec but not yet implemented; the
schema and signatures support them.

- **HTTP/3 production cert flags**. ✅ Landed 2026-05-08; see "HTTP/3
  polish pass" above.
- **HTTP/3 streaming RPC bridge**. ✅ Landed 2026-05-08; see "HTTP/3
  polish pass" above.
- **Typed `ErrorDetail` carrier**. Errors today encode the OHDC code as a
  message prefix (`"OUT_OF_SCOPE: ..."`); a follow-up pass packs a typed
  `ErrorDetail` (proto3 `ErrorInfo`) so generic Connect / gRPC clients
  can read it as structured data, not by string-splitting the message.
  `connectrpc 0.4`'s `ConnectError::with_detail(ErrorDetail)` is the
  insertion point.
- **AuthService / RelayService trait impls**. `SyncService` landed in this
  pass (`build.rs` compiles `sync.proto`). `AuthService` and `RelayService`
  are intentionally still uncompiled; their handlers are deferred per the
  OIDC + relay design docs (Auth lives in the consumer apps' OIDC dance,
  Relay is owned by the relay-hardening agent).
- **~~`AttachBlob` / `ReadAttachment` / `ReadSamples`~~**. ✅ Wired in the
  2026-05-09 sweep. Sidecar attachments live under
  `<storage_dir>/attachments/<sha[..2]>/<sha>`; ReadSamples streams 1024-
  sample batches; AttachBlob caps at 50 MiB.
- **~~Sample-block codecs (encoding 1, 2)~~**. ✅ Landed.
  Determinism + zstd-level-3 pinning + 11 unit tests + 3 corpus fixtures.
- **Filter-language compiler — partial**. v1 now supports `from_ms`,
  `to_ms`, `event_types_in`, `event_types_not_in`, `include_deleted`,
  `include_superseded`, `limit`, `device_id_in`, `source_in`,
  `event_ulids_in`, `sensitivity_classes_in/_not_in`, and
  `channel_predicates` (AND-of, with `eq`/`neq`/`gt`/`gte`/`lt`/`lte`).
  Channel predicates run as a post-query filter pass over the events that
  pass the cheap predicates — perf trade-off documented in
  `events::query_events`'s doc comment. SQL-native channel-predicate
  evaluation is a v1.x optimization once the conformance corpus is
  executable. `text_contains` (case-insensitive substring) and
  multi-ordinal `enum_in` predicates are still deferred.
- **~~Full grant resolution algorithm — partial~~**. ✅ Landed in the
  2026-05-09 sweep. Per-channel filtering, sensitivity-class deny against
  channel-level `sensitivity_class`, time windows (rolling + absolute),
  rate limits (`max_queries_per_day` / `_per_hour` against `audit_log`),
  and the full precedence ladder are wired. Tests live in
  `pending_grants.rs::resolver_*`. **Still stubbed**:
  `require_approval_per_query` (needs the per-query approval-queue table
  + Connect UI flow; documented in `spec/privacy-access.md` Operation-level
  scope).
- **Pending-event flow — sweep**. The wire trio (`ListPending`,
  `ApprovePending`, `RejectPending`) is wired and exercised end-to-end
  via Connect-RPC. The auto-expire sweep (`pending::sweep_expired`) is
  implemented but not yet hooked to a periodic job — operators run it
  manually for now (or the OHDC `ListPending` caller filters by `status`).
- **~~Cases CRUD + scope resolution~~**. ✅ All nine `Case*` RPCs wired
  (Create/Update/Close/Reopen/List/Get + Add/Remove/List CaseFilter). Scope
  resolution + `grant_cases` binding lives in
  `crates/ohd-storage-core/src/cases.rs`.
- **~~Sync (cache ↔ primary)~~**. ✅ Wired in the 2026-05-09 sweep.
  `SyncService.Hello` + `PushFrames` + `PullFrames` + `*GrantOnPrimary`
  delegations. Per-peer rowid watermarks, ULID dedup,
  `origin_peer_id`-based echo suppression. Tests in
  `pending_grants.rs::sync_*`. Attachment payload sync
  (`Push/PullAttachmentBlob`) is the lone deliberate stub — the consumer
  `OhdcService.AttachBlob` covers the same blob store; sync-side delivery
  is a v1.x polish-pass deliverable.
- **Encryption hierarchy**. SQLCipher `PRAGMA key` works against the raw 32
  bytes the caller supplies. The `K_file` / `K_envelope` / `K_recovery`
  three-layer hierarchy from `spec/encryption.md` (BIP39 phrase, online key
  rotation, wrapped-keys table) is v1.x.
- **AuthService / RelayService**. OHDC's `OhdcService` is fully wired and
  `SyncService` landed in the 2026-05-09 sweep. AuthService / RelayService
  are scaffolded in `proto/` but have no Rust handlers yet (see "What's
  stubbed" above for rationale).
- **Bindings**. ✅ `ohd-storage-bindings` now ships **two** layered
  binding facades on the same Rust source crate:
  - **uniffi 0.28** for Kotlin (Android) and Swift (iOS) — see "Bindings
    (2026-05-08)" above. The cross-build recipe lives at
    `connect/android/BUILD.md`; iOS's `.xcframework` recipe is a small
    extension covered by the README.
  - **PyO3 0.28** for Python (CPython 3.11+) — landed 2026-05-09. New
    `crates/ohd-storage-bindings/src/pyo3_module.rs` exposes the same
    surface (`OhdStorage`, `EventInputDto`, `EventFilterDto`, `EventDto`,
    `PutEventOutcomeDto`, `ChannelValueDto`, `ValueKind`) as Python
    classes plus a five-class exception hierarchy rooted at `OhdError`
    (`OpenFailed` / `Auth` / `InvalidInput` / `NotFound` / `Internal`).
    Long-running ops release the GIL via `Python::detach`. Wheel built
    with `maturin build --release` (cargo features `pyo3,extension-module`
    are baked into `pyproject.toml`); the resulting `abi3-py311` wheel
    covers every CPython 3.11+ on a target platform with one binary.
  - **Per-platform packaging.** Maven `.aar` for Android and signed
    `.xcframework` for iOS are still v1.x. The Python wheel is the first
    binding artifact that ships end-to-end (`maturin build` → install
    → import → pytest); see `crates/ohd-storage-bindings/README.md`.
  - **Conformance via the wheel.** New `crates/ohd-storage-conformance/
    run_corpus.py` drives the corpus through the PyO3 wheel as a
    cross-check against the Rust runner — sample-block byte-determinism
    fixtures stay on the Rust side (codec lives in `ohd-storage-core`),
    `ohdc/put_query/*` runs through the wheel, and `permissions/*` skips
    cleanly because grant scope isn't on the binding surface (the
    Connect-RPC path covers that wire-side).
- **HTTP-only OAuth/discovery endpoints**
  (`/.well-known/oauth-authorization-server`, `/authorize`, `/token`,
  `/oauth/register`, etc.) are v1.x.
- **~~Conformance harness~~**. ✅ Landed in the 2026-05-09 sweep.
  `crates/ohd-storage-conformance/` ships the runner + a five-fixture seed
  corpus. `cargo test -p ohd-storage-conformance corpus_passes` runs all
  fixtures byte-for-byte against a fresh storage instance. Categories
  `sync/*`, `auth/*`, `streaming/*` are documented in `spec/conformance.md`
  but not yet seeded — adding fixtures is "drop input.json + expected.json
  + README.md + manifest entry" per the lib doc-comment.

## Decisions and deviations from the pinned stack

These are choices made during implementation that the user may want to flag or
redirect.

1. **Wire transport: real Connect-RPC (binary Protobuf) over HTTP/2 and
   HTTP/3.** The wire is `connectrpc 0.4` + `buffa 0.5` + `hyper`
   (HTTP/1.1 + HTTP/2) and `quinn 0.11` + `h3 0.0.8` + `h3-quinn 0.0.10`
   (HTTP/3). Codec is per-request (`application/proto`,
   `application/grpc+proto`, or `application/json`); the e2e tests use
   gRPC binary framing on HTTP/2 and Connect-protocol binary framing on
   HTTP/3 (Connect's body-envelope status sidesteps h3's trailer
   immaturity). Both listeners share one `ConnectRpcService` so handler
   bodies are identical regardless of transport. See "HTTP/3 (in-binary)
   — landed" at the top of this file for full details.
2. **Token store inside the per-user file, not in a separate system DB.**
   `spec/auth.md` puts `oidc_identities`, `sessions`, `pending_invites`, and
   token rows in a deployment-level system DB (so multi-tenant deployments
   can share OAuth state across users). v1 colocates a `_tokens` table inside
   the per-user file to keep the smoke test self-contained — one file is the
   complete deployment. **Pickup**: when the deployment story lands, the
   token resolver moves into a system-DB-backed `auth::TokenStore` trait;
   the per-user file gets a thin lookup that joins by user ULID.
3. **Storage handle uses a single `std::sync::Mutex`.** SQLite WAL allows
   many readers + one writer, but the rusqlite `Connection` is `!Sync`. v1
   serializes everything through one mutex to keep the implementation
   compact. The right move is a pool with a writer and a `RwLock`-style
   reader-borrow pattern (or `r2d2` + per-connection PRAGMA setup); easy
   refactor.
4. **`base32` (RFC 4648) for token bodies, NOT `base64url`.** The spec
   examples show `ohds_<base64url>`. Rust's `base32` crate landed cleanly
   as a workspace dep; switching to `base64url` is a one-line change in
   `auth::mint_token_body`.
5. **Standard registry ID assignment is autoincrement.** `spec/data-model.md`
   says "stable IDs in the embedded registry catalog." The current seed
   relies on the runtime registry resolving by `(namespace, name)` rather
   than by frozen integer IDs. **Pickup**: a future migration freezes the
   IDs with explicit `INSERT … (id, …)` once the catalog stabilizes; needed
   before exports cross deployments.
6. **`Cargo.lock` regenerated**. The scaffolding lockfile mentioned only
   anyhow/clap/thiserror; the implementation pulls in tokio + axum + rusqlite
   + bundled sqlcipher + sha2 + zeroize + base32 + serde. First build does
   the full SQLCipher C compile (~3 minutes); incremental builds finish in
   seconds.
7. **MSRV bump 1.83 → 1.88**. Required by `connectrpc 0.4` / `buffa 0.5`,
   which use edition-2024 features in their generated code. The
   `rust-toolchain.toml` still pins `1.83.0` for backwards-compatibility
   reasons; environments without `rustup` (Arch's system rust at 1.94 in
   our case) fall through to whatever the system ships, which works.
8. **Build-script protoc**. The codegen path uses
   `protoc-bin-vendored` to provide a `protoc` binary at build time
   instead of requiring a system install. This adds ~5MB of native
   binaries to the build cache but keeps `cargo build` self-contained on
   any host. Switching to `buf generate` or precompiled descriptor sets
   is a one-liner via `Config::use_buf()` / `Config::descriptor_set()`.

## Suggested next-phase order

1. **HTTP/3 production cert flags** + **streaming-body bridge**. The
   in-binary HTTP/3 listener is working (`--http3-listen` flag). Pickup:
   `--http3-cert` / `--http3-key` PEM loaders, plus an `http_body::Body`
   wrapper around `h3::server::RequestStream` so the streaming RPCs
   (`AttachBlob`, `Import`, `QueryEvents`, …) don't go through the 4 MiB
   buffer cap.
2. **Typed `ErrorDetail` carrier**. Replace the `"OHDC_CODE: msg"`
   prefix convention with a structured `ErrorInfo` packed into
   `connectrpc::error::ErrorDetail`.
3. **Full grant resolution algorithm (P3)**. Per-channel filtering,
   sensitivity-class deny against the channel's own `sensitivity_class`,
   time windows (rolling + absolute), rate limits (`max_queries_per_day`
   / `_per_hour`). Schema rows already populated by `CreateGrant`; the
   resolver in `events::query_events` needs to consume them. Anchor the
   work on the conformance-corpus fixtures.
4. **Pending sweep daemon**. Wire `pending::sweep_expired` to a periodic
   tokio task on the server that flips expired rows; today operators must
   run a manual sweep (or `ListPending` callers filter by `status`).
5. **SQL-native channel predicates**. Today the predicate evaluation runs
   as a post-query filter pass in Rust; once we have the conformance
   corpus, push the cheap operators (`eq`/`gt` against indexed channels)
   into the SQL `WHERE`.
6. Sample-block codecs + `ReadSamples` / `AttachBlob` / `ReadAttachment`.
7. Cases CRUD + case-scope resolver.
8. `AuditQuery` server-streaming handler + `Aggregate` / `Correlate`.
9. Sync (cache + primary, grant-on-primary RPCs) — compile `sync.proto`
   and add a second `register()` call alongside `OhdcService`.
10. Three-layer encryption hierarchy + BIP39 recovery + online rotation.
11. ~~Bindings (uniffi + PyO3).~~ ✅ landed; see "Bindings (2026-05-08)"
    (uniffi facade) and "Bindings" in "What's stubbed / deferred"
    (2026-05-09 PyO3 + maturin Python wheel). Pickup: per-platform
    packaging for the mobile artefacts — Maven `.aar` publishing for
    Android and a signed `.xcframework` for iOS (the Python wheel ships
    end-to-end already).
12. Conformance harness.
