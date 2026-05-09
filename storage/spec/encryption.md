# Design: Encryption & Key Management

> The crypto layer for OHD Storage. Per-user file encryption, recovery, multi-device pairing, key rotation, export encryption, and the placeholder for end-to-end channel encryption. Pairs with [`storage-format.md`](storage-format.md) (which mandates SQLCipher 4 + per-user key but doesn't fully spec how the key is derived, recovered, or rotated).

## Threat model recap

From [`privacy-access.md`](privacy-access.md), the threats this doc must defend against:

| Threat | Defense |
|---|---|
| Lost / stolen device with encrypted file | Per-user key derived from a secret the device doesn't store in the clear (passphrase + biometric-unlocked Keystore item) |
| Compromised SaaS operator wanting to read PHI without grants | Per-user key wrapped in a way that requires the user's secret — operator can't derive it from disk alone |
| Lost passphrase | Recovery secret (BIP39) → key re-derivation; lose both → data loss (clearly disclosed) |
| Stolen backup blob (offsite restore copy leaked) | Same per-user-key encryption protects backups; restoration without the user secret yields ciphertext |
| Subpoena to a SaaS operator demanding plaintext | Operator can't comply for sensitive sensitivity classes if e2e channel encryption is in use (forward-deferred); for general data the operator complies under jurisdictional law (acknowledged limitation) |
| Multi-device user wants to add a new device without re-entering everything | Wrapped-key handoff via QR + ECDH — old device hands the new one a key wrapped to its public key |
| User wants to switch deployment modes (on-device → cloud, etc.) | Migration tool re-derives / re-wraps the file key for the destination environment |

## Key hierarchy

Three keys per user, distinct in role:

| Key | Symbol | Purpose | Lifetime |
|---|---|---|---|
| **File key (FK)** | `K_file` | The actual SQLCipher 4 page-encryption key (and the libsodium key for sidecar blobs). 256 bits. | Per-user, long-lived (rotates on demand). |
| **Envelope key (EK)** | `K_envelope` | Encrypts the file key when it has to be stored (server-side or as a backup wrap). 256 bits. Derived from a user-held secret + per-file salt. | Re-derived on every unlock. |
| **Recovery key (RK)** | `K_recovery` | Alternate way to derive `K_envelope` if the user's primary unlock secret is lost. Encoded as a BIP39 phrase shown once at first launch. | Per-user, long-lived. |

The file key never leaves the storage process in plaintext. It's derived (or unwrapped) at unlock time, held in process memory, and zeroed when the process exits or the user explicitly locks.

## Per-deployment-mode key flow

### On-device Mode A (OIDC-bound) and Mode B (anonymous)

The file lives only on the user's device. Unlock requires either the user's passphrase or a platform-secured biometric-released key.

**First launch:**

1. User picks a passphrase (Mode B) or completes OIDC + picks an unlock passphrase / biometric (Mode A).
2. Generate `K_file` via CSPRNG (256 bits).
3. Generate per-file salt `S_kdf` (128 bits, stored in `_meta.cipher_kdf`).
4. Derive `K_envelope = KDF(passphrase, S_kdf)` — see "KDF choice" below.
5. Encrypt and store the wrapped file key: `WrappedFK_pass = AEAD(K_envelope, K_file, nonce_pass)`. Stored in `_meta.wrapped_file_keys` (a small embedded table — schema below).
6. If biometric is enabled, additionally store `WrappedFK_bio = AEAD(K_bio, K_file, nonce_bio)`, where `K_bio` is a key generated and stored in the platform Keystore/Keychain bound to biometric release. The OS prevents `K_bio` from ever leaving the secure enclave.
7. Generate a 24-word **BIP39 recovery phrase**; derive `K_recovery = BIP39_to_seed(phrase)` (BIP39 standard PBKDF2).
8. Store `WrappedFK_recovery = AEAD(K_recovery, K_file, nonce_rec)`.
9. **Show the recovery phrase to the user once.** UI stresses: write it down or save in a password manager; if you lose your passphrase AND this phrase, your data is gone.

**Subsequent launches:**

- Biometric path: OS releases `K_bio` after biometric → unwrap `K_file = AEAD_open(K_bio, WrappedFK_bio, nonce_bio)`.
- Passphrase fallback: user enters passphrase → derive `K_envelope = KDF(passphrase, S_kdf)` → unwrap `K_file = AEAD_open(K_envelope, WrappedFK_pass, nonce_pass)`.
- Recovery path: user enters BIP39 phrase → derive `K_recovery` → unwrap `K_file = AEAD_open(K_recovery, WrappedFK_recovery, nonce_rec)`.

`K_file` is loaded into the Rust storage core's memory, used as the SQLCipher key (`PRAGMA key`) and the libsodium blob-stream key. On lock / process exit, the in-memory key is zeroed (`zeroize` crate).

### Cloud / custom-provider / self-hosted-server

The file lives on the operator's infrastructure. The operator must not be able to decrypt without the user's involvement (for the sensitivity-class story to be defensible). So:

**First launch:**

1. User completes OIDC, picks an unlock passphrase.
2. Generate `K_file` on the user's device (Connect mobile / web).
3. Generate per-file salt `S_kdf` server-side (in `_meta.cipher_kdf` of the new file).
4. Connect derives `K_envelope = KDF(passphrase, S_kdf)` locally.
5. Connect computes `WrappedFK_pass = AEAD(K_envelope, K_file, nonce_pass)` locally.
6. Connect uploads the **wrapped key only** to the server, plus per-device wraps and the recovery wrap as in Mode A.
7. Server stores the wraps in the user's `_meta.wrapped_file_keys` table.
8. Server **never sees `K_file` or the passphrase**.

**Subsequent unlocks:**

- Connect prompts for passphrase (or releases biometric → cached envelope key). Derives `K_envelope`.
- Connect downloads `WrappedFK_pass` (and `S_kdf` if not cached).
- Connect unwraps `K_file` locally.
- Connect transmits `K_file` to the server **only inside the OHDC TLS session** as a per-session unlock RPC: `Auth.UnlockFile(K_file)`. Server uses it to open the user's file for the duration of the session, zeros it on session end.

**Why send `K_file` per session rather than wrapping it server-side every time:** SQLCipher needs the raw key to operate; the server's read/write paths are inside the user's already-authenticated TLS session anyway. The cost is "the server has the key in memory while the session is active." That's an acceptable damage cap (a session-attacker already has session access). The win is the operator never has the key at rest, never logs it, and a backup-blob leak is useless without the user.

For OHD Cloud specifically, an optional opt-in **operator-side recovery escrow** is offered with very loud disclosure UX: "OHD Cloud will hold a copy of your encryption key. If you lose your passphrase, we can recover your data — but it also means OHD Cloud could be compelled to decrypt your data under subpoena. Most users should NOT enable this." Off by default.

### Hybrid: phone primary + cloud cache (or vice versa)

The file is the same file in both places (sync replays events; the SQLCipher key is the same). `K_file` is generated wherever the file is first created (typically the phone in this topology). When the cloud cache is set up, the user does the wrap-and-upload step once; from then on, the cloud cache works the same as a cloud-primary deployment for unlock.

## KDF choice

Memory-hard, modern, future-proofs against GPU/ASIC attacks.

| Where | KDF | Parameters |
|---|---|---|
| Today (SQLCipher 4 era) | **PBKDF2-SHA512** | 256k iterations (SQLCipher 4 default). Same KDF SQLCipher uses internally; aligns with the engine's expectations. |
| Future (SQLCipher 5 lands) | **Argon2id** | 64 MiB memory, 3 iterations, 1 lane. Migration path: re-derive `K_envelope` from passphrase + per-file salt under Argon2id, re-wrap the file key, update `_meta.cipher_kdf` to record the algo. Transparent to data; one-shot per file. |

KDF parameters are stored in `_meta.cipher_kdf` as a structured row: `algorithm`, `iterations` (or `memory_cost` / `time_cost` / `parallelism` for Argon2id), `salt`, plus a `wrap_format_version` for forward-compat. The unlock code reads these and runs the appropriate KDF; old files keep working after a library upgrade.

## Wrapped-key storage (in-file)

A small embedded table holds all wraps. Lives in the per-user file (so it travels with the file across migrations).

```sql
CREATE TABLE _meta_wrapped_file_keys (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  wrap_kind       TEXT NOT NULL,          -- 'passphrase' | 'biometric_<device_id>' | 'recovery' | 'escrow' | 'pair_<device_id>'
  device_label    TEXT,                   -- "Jakub's iPhone 15", user-editable
  wrap_format     TEXT NOT NULL,          -- 'aead_xchacha20poly1305' (v1 default)
  ciphertext      BLOB NOT NULL,          -- the wrapped K_file
  nonce           BLOB NOT NULL,          -- AEAD nonce (24 bytes for XChaCha20)
  created_at_ms   INTEGER NOT NULL,
  last_used_ms    INTEGER,
  retired_at_ms   INTEGER                 -- non-null = wrap no longer accepted (after rotation grace period)
);

CREATE INDEX idx_wraps_kind ON _meta_wrapped_file_keys (wrap_kind);
```

A user can have multiple wraps active simultaneously: passphrase + biometric on phone + biometric on iPad + recovery + (optional) escrow. Adding a wrap doesn't change `K_file`; it only adds a new way to reach it.

## Multi-device pairing

User wants to use Connect on a second device (laptop, tablet, new phone) without re-entering passphrase + recovery.

**Pairing flow:**

1. New device generates an ephemeral X25519 keypair `(EK_priv_new, EK_pub_new)`.
2. New device displays a QR code containing `(EK_pub_new, device_label_proposed, pairing_nonce)`.
3. Existing device scans the QR. UI confirms "Pair Jakub's iPad?" with biometric / passphrase.
4. Existing device computes `K_pair = X25519(EK_priv_existing_device_session, EK_pub_new)` (uses an ephemeral key from the existing device's side too).
5. Existing device wraps the file key: `WrappedFK_pair = AEAD(K_pair, K_file, nonce)`.
6. Existing device sends `WrappedFK_pair` + its `EK_pub_existing_session` back to the new device (via local Bluetooth, mDNS, or a relay-mediated short-lived session).
7. New device computes `K_pair = X25519(EK_priv_new, EK_pub_existing_session)`, unwraps `K_file`.
8. New device stores its own biometric wrap of `K_file` in the file's `_meta_wrapped_file_keys` (writes back via OHDC under a paired-device session, since the new device now has the file key).
9. Both ephemeral private keys are zeroed.

The QR / pairing channel needs to be authentic enough to prevent MITM — physical proximity (BLE / NFC / camera scan) is the trust anchor. Relay-mediated pairing for "I'm migrating to a new phone after losing the old one but I have it briefly" can use the rendezvous URL + grant-style auth, but the OS-level QR-scan path is the primary recommendation.

## Key rotation

The file key can be rotated on demand (after a suspected compromise, on a schedule, or as part of a security policy):

1. Generate `K_file_new` via CSPRNG.
2. Re-encrypt every page of the file via SQLCipher's `PRAGMA rekey`. This is a single-pass rewrite; for a 50 GB file it can take an hour. During rotation, the file is locked (no writes); reads continue against the previous key until the rotation completes.
3. Re-wrap with every active wrap entry: passphrase, biometric (each device), recovery, escrow if enabled.
4. Mark old wraps `retired_at_ms`; remove after a 7-day grace period (gives all the user's devices time to discover the new wraps and update their cached `K_file`).
5. Sidecar blobs are re-encrypted lazily — each blob is independently encrypted under `K_file`, so they all need re-encryption. Either eagerly (rewrite all blobs after the file rekey) or lazily (rewrite each blob on next access). Operator picks based on data volume.

Audit: rotation produces an audit row `actor_type='self'` `action='key_rotated'`. Suspicious-circumstances rotations get a `reason` field set.

The user should be prompted to **re-do their multi-device pairings** after a rotation initiated due to suspected compromise, since old wraps from compromised devices have to be retired.

## Recovery

The BIP39 recovery phrase is the primary "I forgot my passphrase" backstop. The flow:

1. User taps "Recover with phrase" on first launch on a new device, or "Reset passphrase" in Connect on a current device.
2. Enters the 24-word phrase.
3. App derives `K_recovery` via BIP39 standard PBKDF2-HMAC-SHA512.
4. App downloads `WrappedFK_recovery` from the file, unwraps `K_file`.
5. App prompts the user to set a new passphrase; computes a new `K_envelope`; writes a new `WrappedFK_pass` row to the file. Old passphrase wrap retired.

If the user has neither passphrase nor recovery phrase, the data is unrecoverable. Stated explicitly in the first-launch UX; restated whenever the user reaches a danger point (changing devices, factory reset, etc.).

## Export encryption

Exports default to **plaintext** (so importers can ingest without coordination). Optional per-export passphrase encryption for users handing the file to a third-party for backup:

- User picks "Export with password" → app prompts for a one-time passphrase.
- App generates `K_export = KDF(passphrase, fresh_salt)`.
- Export file body is AEAD-encrypted with `K_export`; salt and KDF params travel in the export envelope header (cleartext).
- Importer prompts for the passphrase, derives `K_export`, decrypts.

Exports are signed by the source instance's identity key (separate from `K_file`, see "Identity key" below). Receiving instances verify the signature when they trust the source.

The export's *content* always uses the canonical OHDC export format — encryption is a wrapper around it, not a different format.

## Identity key (per storage instance)

Separate from the per-user file key: each storage instance has an **Ed25519 identity keypair** generated on first deployment and stored under operator-controlled secret management (KMS / HSM / passphrase-protected file). Used for:

- Signing exports (so receiving instances can verify provenance).
- Authenticating the storage to its relay (the relay uses the public key to verify long-lived tunnel registration).
- TLS server-cert generation when the storage is on-device and uses self-signed certs through a relay (see [`../components/relay.md`](../components/relay.md) "Persistence" — the certificate's public key is derived from this identity key).

Identity keys are durable per instance; rotating one means re-issuing every grant the user has out (the cert pin in the grant artifact references the public key) and re-registering with the relay. Treat as rare event.

## End-to-end channel encryption (deferred)

For the most sensitive sensitivity classes (`mental_health`, `sexual_health`, `substance_use`, `reproductive`), we want **operator-cannot-read** even at the engine level. That requires a separate user-held key (`K_e2e`) that encrypts the relevant cells before they hit SQLCipher — even if the operator has the SQLCipher key, the cell content is still ciphertext.

Reserved in this doc but not yet specced at the bit level. Open items for the future:

- **Wrap format**: how `K_e2e` is stored (similar `_meta_e2e_wrapped_keys` table) and how it gets shared with grantees who are explicitly allowed sensitivity-class access.
- **Per-grant key sharing**: when the user issues a grant with `mental_health` access, they have to wrap `K_e2e` to the grantee's public key and store that wrap somewhere the grant can find it.
- **Cell-level cryptography**: which fields are encrypted (`event_channels.value_*` columns for sensitive channels? Whole event rows? Just notes?).
- **Search and query**: operator can't index encrypted content, so queries against e2e fields are client-side only or use deterministic-encryption tricks (which weaken the contract).

This is genuine new design work. Tracked as future-implementations entry: `future-implementations/e2e-channel-encryption.md` (TBD).

## In-transit encryption (recap)

Not strictly key-management but worth pinning: TLS 1.3 required for all OHDC traffic. Caddy handles termination. End-to-end through OHD Relay (relay sees ciphertext only; storage and client terminate TLS themselves). Cert pinning in grant artifacts. Details in [`../components/relay.md`](../components/relay.md) "Trust model" and the forthcoming Relay TLS-through-tunnel spec (Task #11).

## Cross-references

- On-disk SQLCipher mandate: [`storage-format.md`](storage-format.md) "Encryption"
- Auth and session tokens (separate concern): [`auth.md`](auth.md)
- Care-side token vault encryption (uses operator KMS, not per-user keys): [`care-auth.md`](care-auth.md)
- Privacy / threat model: [`privacy-access.md`](privacy-access.md)
- Relay TLS-end-to-end model: [`../components/relay.md`](../components/relay.md)

## Open items (forwarded to other tasks)

- **End-to-end channel encryption** for sensitive sensitivity classes — placeholder above; full spec deferred. Will move to `future-implementations/e2e-channel-encryption.md`.
- **TLS server-identity model for relay-mediated storage** — depends on Task #11 (Relay TLS-through-tunnel cert/identity).
- **Operator KMS recommendations per deployment scale** — partly covered in [`care-auth.md`](care-auth.md) "Token storage on the Care server"; could promote to a deployment-wide KMS-recommendations doc later.
