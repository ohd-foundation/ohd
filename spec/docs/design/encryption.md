# Design: Encryption & Key Management

> The shipped OHD Storage encryption model. This document is byte-level
> normative for cross-implementation decryptors, verifiers, and grant-wrap
> tooling.

## Threat Model Recap

| Threat | Defense |
|---|---|
| Lost or stolen storage file | SQLCipher page encryption under `K_file`. |
| Operator reads highly sensitive channels from a live database | Per-class value encryption for selected sensitivity classes. |
| Lost primary unlock route | 24-word BIP39 recovery mnemonic derives the same storage key material. |
| Row-level ciphertext swapping | AEAD AAD binds ciphertext to channel/attachment/grant identity. |
| Cross-storage grants | X25519 ECDH re-targets wrapped class keys to the grantee storage. |

## Key Hierarchy

The shipped hierarchy is:

```text
24-word BIP39 mnemonic
  -> K_recovery seed bytes
       PBKDF2-HMAC-SHA512(password = mnemonic phrase,
                          salt = "mnemonic" || bip39_passphrase,
                          iterations = 2048,
                          output = 64 bytes)
  -> K_file
       HKDF-SHA256(ikm = K_recovery,
                   salt = _meta.k_recovery_salt,
                   info = "ohd.v0.file_key",
                   output = 32 bytes)
  -> SQLCipher PRAGMA key
  -> K_envelope
       HKDF-SHA256(ikm = K_file,
                   salt = empty,
                   info = "ohd.v0.envelope_key",
                   output = 32 bytes)
  -> K_class[class]
       random 32-byte DEK per encrypted sensitivity class,
       wrapped under K_envelope in class_keys/class_key_history
```

`K_file` is the SQLCipher key. `K_envelope` never wraps `K_file`; it is
derived from `K_file` and only wraps per-class data-encryption keys.

`_meta.k_recovery_salt` is generated once at file creation. The BIP39
passphrase parameter is currently empty in product flows; it remains part of
the derivation for compatibility with the BIP39 seed function.

## Class Keys

The default encrypted classes are:

- `mental_health`
- `sexual_health`
- `substance_use`
- `reproductive`

For each class, storage keeps a live `K_class[class]` in `class_keys`, with
rotation history in `class_key_history`. Each history row is an AES-256-GCM
wrap of the 32-byte class DEK under `K_envelope`. `class_keys.current_history_id`
points at the active generation. Channel rows store
`event_channels.encryption_key_id = class_key_history.id` so old rows remain
decryptable after class-key rotation.

## Channel Value AEAD

Encrypted channel values use XChaCha20-Poly1305. The on-disk
`event_channels.value_blob` layout is:

```text
nonce[24] || ciphertext || tag[16]
```

The plaintext is CBOR of the tagged scalar:

```text
{ "k": "real"|"int"|"bool"|"text"|"enum", "v": value }
```

The AAD is constructed byte-for-byte as:

```text
"ohd.v0.ch:" || utf8(channel_path)
|| "|evt:" || event_ulid_bytes_16
|| "|key:" || little_endian_i64(encryption_key_id)
```

There is one shipped path: XChaCha20-Poly1305 with the AAD above. There is no
V1 path, no `aad_version` dispatch, and no AES-GCM channel-value fallback.
This is backward-incompatible by design because the format is pre-deployment.

## Attachment AEAD

Attachment payloads are encrypted as sidecar blobs using
XChaCha20-Poly1305 STREAM-BE32 with 64 KiB chunks. Attachment metadata keeps
the encrypted attachment DEK wrap (`wrapped_dek`, `dek_nonce`) plus the
metadata needed for AAD reconstruction.

The attachment payload AAD is:

```text
"ohd.v0.att:" || attachment_ulid_bytes_16
|| "|evt:" || event_ulid_bytes_16
|| "|sha:" || sha256_bytes_32
|| "|mime:" || utf8(mime_type)
|| "|name:" || utf8(filename)
|| "|sz:" || little_endian_u64(byte_size)
```

The SHA-256 is over the plaintext attachment bytes. The filename and MIME type
are the exact strings stored for the attachment; empty strings are encoded as
zero bytes after their labels.

## Multi-Storage Grant Re-Targeting

When a grant includes encrypted classes and the grantee has a separate storage
file, class keys are re-targeted with X25519:

```text
shared = X25519(K_recovery_seckey_issuer, K_recovery_pubkey_grantee)
kek = HKDF-SHA256(
  ikm = shared,
  salt = "ohd.v0.grant_kek",
  info = "ohd.v0.grant_kek|" || utf8(class)
         || "|iss:" || issuer_pubkey_32
         || "|grt:" || grantee_pubkey_32,
  output = 32 bytes
)
wrapped = AES-256-GCM(kek, plaintext = K_class[class], aad = grant_wrap_aad)
```

The grant-wrap AAD is:

```text
"ohd.v0.grantwrap:" || grant_ulid_bytes_16
|| "|class:" || utf8(class)
|| "|key_id:" || little_endian_i64(class_key_history_id)
```

The wrap is stored in `grants.class_key_wraps`, keyed by sensitivity class,
with the per-class `key_id`. The issuer recovery pubkey is stored in
`grants.issuer_recovery_pubkey`; the target pubkey supplied by the grantee is
stored in `grants.grantee_recovery_pubkey`.

Low-order X25519 public keys are rejected before HKDF by checking for an
all-zero shared secret. Single-storage grants without a grantee recovery
pubkey continue to wrap class keys under the issuer's `K_envelope`.

## Source Signing Canonicalization

Signed event submissions use `EventInput.source_signature`. Storage supports
`ed25519`, `rs256`, and `es256` signers registered in the `signers` table.
Accepted signatures create an `event_signatures` row and queried events return
`Event.signed_by`.

The signed payload is deterministic CBOR of:

```text
{ "u": event_ulid_bytes_16,
  "t": timestamp_ms_i64,
  "e": event_type_text,
  "c": [ { "p": channel_path_text, "v": tagged_scalar }, ... ] }
```

Channels are sorted lexicographically by path. Signed events reject
non-finite floats (`NaN`, `Inf`, `-Inf`) and reject duplicate channel paths
before canonical CBOR is produced.

## Versioning

All domain-separation strings use `ohd.v0.*`. The storage format intentionally
does not carry legacy V1 decryption branches: pre-deployment data can be
recreated, and cross-implementation verifiers get one clean format to
implement.

## Cross-References

- On-disk schema: [`storage-format.md`](storage-format.md)
- OHDC wire messages: [`ohdc-protocol.md`](ohdc-protocol.md)
- Access-control model: [`privacy-access.md`](privacy-access.md)
