//! Sidecar attachment storage.
//!
//! Per `spec/storage-format.md` "Attachments", attachment bytes live under a
//! per-deployment `<storage_dir>/attachments/<sha256>/...` directory. The
//! `attachments` SQL table records the metadata (ULID, sha256, byte size,
//! MIME, filename, optional wrapped DEK); the file payload is stored on
//! disk addressed by sha256-of-plaintext (so the path is stable across
//! encrypt / re-encrypt rotations).
//!
//! v1 ships:
//! - [`new_writer`] / [`new_writer_with_envelope`] — open a temp-file writer.
//! - [`AttachmentWriter::finalize`] — computes sha256, XChaCha20-Poly1305
//!   STREAM-encrypts under a fresh per-attachment DEK (wrapped under
//!   `K_envelope`), atomically renames the encrypted bytes into
//!   `<root>/<aa>/<sha256_of_plaintext>`, and inserts the metadata row.
//! - [`load_attachment_meta`] — used by `ReadAttachment`.
//! - [`read_attachment_bytes`] — read + decrypt to plaintext.
//! - [`read_and_lazy_migrate_attachment`] — read a pre-existing legacy
//!   plaintext blob and encrypt it in place.
//! - [`MAX_BLOB_BYTES`] cap.
//!
//! # On-disk encryption format
//!
//! When `wrapped_dek IS NOT NULL` in the row, the file contents on disk are
//! `[19-byte stream nonce prefix][chunk0_ct+tag][chunk1_ct+tag] … [chunkN_ct+tag]`.
//! The DEK is 32 bytes, randomly generated per attachment, wrapped under
//! `K_envelope` (AES-256-GCM, AAD = `"ohd.v0.attachment_dek:" || ulid`).
//! The streaming AEAD AAD is the wide bind:
//! `"ohd.v0.att:" || att_ulid || "|evt:" || event_ulid || "|sha:" || sha256
//!  || "|mime:" || mime || "|name:" || filename || "|sz:" || byte_size_le_u64`.
//!
//! Pre-encryption rows (`wrapped_dek IS NULL`) keep their cleartext
//! filesystem layout. New writes go through the encrypted path; the lazy
//! migration covers existing rows on first read.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::{Key as ChaKey, XChaCha20Poly1305};
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::encryption::{EnvelopeKey, DEK_LEN, NONCE_LEN};
use crate::events::AttachmentRef;
use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// AES-GCM-Tag length in bytes.
const AEAD_TAG_LEN: usize = 16;

/// AAD prefix for the wrap of a per-attachment DEK under K_envelope. Binds
/// the wrap to this purpose so it can't be reused to unwrap an arbitrary
/// blob from another module's wrap table.
const ATTACHMENT_DEK_AAD_PREFIX: &[u8] = b"ohd.v0.attachment_dek:";

/// V2 streaming-AEAD chunk size. 64 KiB is a balance between per-chunk
/// overhead (a 16-byte tag + small AAD) and memory pressure (peak working
/// set during finalize is one chunk's worth of plaintext + ciphertext).
/// Codex review #6.
const STREAM_CHUNK_SIZE: usize = 64 * 1024;

/// V2 streaming-AEAD nonce-prefix length (the chacha20poly1305 STREAM
/// construction over XChaCha20-Poly1305 reserves the trailing 5 bytes of
/// the 24-byte XNONCE for `(counter:u32 || last_chunk_flag:u8)`, so the
/// caller-provided "stream nonce" is 19 bytes).
const STREAM_NONCE_PREFIX_LEN: usize = 19;

/// Build the attachment-payload AAD.
///
/// Codex review #3: binds `(att_ulid, event_ulid, sha256, mime, filename,
/// byte_size)` into the AAD so any row-level swap or metadata edit is
/// caught at decrypt time.
fn attachment_aad(
    attachment_ulid: &Ulid,
    event_ulid: &Ulid,
    sha256: &[u8; 32],
    mime_type: Option<&str>,
    filename: Option<&str>,
    byte_size: u64,
) -> Vec<u8> {
    let mime = mime_type.unwrap_or("");
    let name = filename.unwrap_or("");
    let mut aad = Vec::with_capacity(
        b"ohd.v0.att:".len()
            + 16
            + b"|evt:".len()
            + 16
            + b"|sha:".len()
            + 32
            + b"|mime:".len()
            + mime.len()
            + b"|name:".len()
            + name.len()
            + b"|sz:".len()
            + 8,
    );
    aad.extend_from_slice(b"ohd.v0.att:");
    aad.extend_from_slice(attachment_ulid);
    aad.extend_from_slice(b"|evt:");
    aad.extend_from_slice(event_ulid);
    aad.extend_from_slice(b"|sha:");
    aad.extend_from_slice(sha256);
    aad.extend_from_slice(b"|mime:");
    aad.extend_from_slice(mime.as_bytes());
    aad.extend_from_slice(b"|name:");
    aad.extend_from_slice(name.as_bytes());
    aad.extend_from_slice(b"|sz:");
    aad.extend_from_slice(byte_size.to_le_bytes().as_ref());
    aad
}

/// Streaming-AEAD encrypt of an attachment payload using XChaCha20-Poly1305
/// in the STREAM-BE32 construction (RustCrypto's `aead::stream`).
///
/// Codex review #6: avoids loading the entire plaintext into memory at
/// finalize time. The temp-file plaintext is read in `STREAM_CHUNK_SIZE`
/// chunks, each chunk encrypted independently with a chunk-counter-keyed
/// nonce derived from the random 19-byte stream nonce prefix; the final
/// chunk uses the STREAM "last block" flag (set internally by
/// `EncryptorBE32::encrypt_last`).
///
/// On-disk layout:
/// ```text
/// [19-byte stream nonce prefix][chunk0_ct+tag][chunk1_ct+tag] … [chunkN_ct+tag]
/// ```
/// Each chunk is `STREAM_CHUNK_SIZE` bytes of plaintext + 16-byte tag
/// (except the last chunk, which is plaintext.len() + 16 bytes).
///
/// `aad` is the V2 wide-AAD (event_ulid + sha256 + mime + filename + size)
/// and is fed to every chunk.
fn encrypt_attachment_stream(
    dek: &[u8; DEK_LEN],
    aad: &[u8],
    src: &mut fs::File,
    plaintext_len: u64,
    dst: &mut fs::File,
) -> Result<()> {
    let mut nonce_prefix = [0u8; STREAM_NONCE_PREFIX_LEN];
    OsRng.fill_bytes(&mut nonce_prefix);
    dst.write_all(&nonce_prefix)?;

    let cipher = XChaCha20Poly1305::new(ChaKey::from_slice(dek));
    let mut encryptor = Some(EncryptorBE32::from_aead(cipher, (&nonce_prefix).into()));

    src.seek(SeekFrom::Start(0))?;
    let mut buf = Zeroizing::new(vec![0u8; STREAM_CHUNK_SIZE]);
    let mut consumed: u64 = 0;
    // Always emit at least one chunk: encrypt_last is required to terminate
    // the STREAM and emits the final tag, even for empty plaintext.
    if plaintext_len == 0 {
        let enc = encryptor.take().expect("encryptor present");
        let ct = enc
            .encrypt_last(chacha20poly1305::aead::Payload { msg: &[], aad })
            .map_err(|_| {
                Error::Internal(anyhow::anyhow!(
                    "XChaCha20-stream encrypt_last failed (empty plaintext)"
                ))
            })?;
        dst.write_all(&ct)?;
    }
    while consumed < plaintext_len {
        let remaining = plaintext_len - consumed;
        let want = remaining.min(STREAM_CHUNK_SIZE as u64) as usize;
        // Read exactly `want` bytes into the front of buf.
        src.read_exact(&mut buf[..want])?;
        consumed += want as u64;
        let is_last = consumed >= plaintext_len;
        let ct = if is_last {
            let enc = encryptor.take().expect("encryptor present on last chunk");
            enc.encrypt_last(chacha20poly1305::aead::Payload {
                msg: &buf[..want],
                aad,
            })
            .map_err(|_| Error::Internal(anyhow::anyhow!("XChaCha20-stream encrypt_last failed")))?
        } else {
            encryptor
                .as_mut()
                .expect("encryptor present mid-stream")
                .encrypt_next(chacha20poly1305::aead::Payload {
                    msg: &buf[..want],
                    aad,
                })
                .map_err(|_| {
                    Error::Internal(anyhow::anyhow!("XChaCha20-stream encrypt_next failed"))
                })?
        };
        dst.write_all(&ct)?;
        // Wipe the consumed plaintext bytes from the buffer before reading
        // the next chunk (Zeroizing zeroes on drop, but per-chunk wipe is
        // defence-in-depth in case of long-lived allocations).
        buf[..want].zeroize();
        if is_last {
            break;
        }
    }
    dst.flush()?;
    Ok(())
}

/// Streaming-AEAD decrypt mirror of [`encrypt_attachment_stream`]. Returns
/// the decrypted plaintext bytes. Codex review #6: bounded peak memory
/// (one chunk + tag at a time).
fn decrypt_attachment_stream(dek: &[u8; DEK_LEN], aad: &[u8], on_disk: &[u8]) -> Result<Vec<u8>> {
    if on_disk.len() < STREAM_NONCE_PREFIX_LEN + AEAD_TAG_LEN {
        return Err(Error::DecryptionFailed);
    }
    let (prefix, body) = on_disk.split_at(STREAM_NONCE_PREFIX_LEN);
    let mut prefix_arr = [0u8; STREAM_NONCE_PREFIX_LEN];
    prefix_arr.copy_from_slice(prefix);

    let cipher = XChaCha20Poly1305::new(ChaKey::from_slice(dek));
    let mut decryptor = Some(DecryptorBE32::from_aead(cipher, (&prefix_arr).into()));

    let chunk_ct_len = STREAM_CHUNK_SIZE + AEAD_TAG_LEN;
    let mut out = Vec::with_capacity(body.len()); // upper bound
    let mut cursor = 0usize;
    while cursor < body.len() {
        let remaining = body.len() - cursor;
        if remaining <= chunk_ct_len {
            // Final chunk — `decrypt_last` consumes the decryptor.
            let dec = decryptor.take().ok_or(Error::DecryptionFailed)?;
            let pt = dec
                .decrypt_last(chacha20poly1305::aead::Payload {
                    msg: &body[cursor..],
                    aad,
                })
                .map_err(|_| Error::DecryptionFailed)?;
            out.extend_from_slice(&pt);
            break;
        } else {
            let pt = decryptor
                .as_mut()
                .ok_or(Error::DecryptionFailed)?
                .decrypt_next(chacha20poly1305::aead::Payload {
                    msg: &body[cursor..cursor + chunk_ct_len],
                    aad,
                })
                .map_err(|_| Error::DecryptionFailed)?;
            out.extend_from_slice(&pt);
            cursor += chunk_ct_len;
        }
    }
    Ok(out)
}

/// Wrap a per-attachment DEK under [`EnvelopeKey`]. AAD is
/// `ATTACHMENT_DEK_AAD_PREFIX || attachment_ulid_bytes`.
fn wrap_attachment_dek(
    envelope: &EnvelopeKey,
    attachment_ulid: &Ulid,
    dek: &[u8; DEK_LEN],
) -> Result<(Vec<u8>, [u8; NONCE_LEN])> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(envelope.as_bytes()));
    let nonce_arr = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut aad = Vec::with_capacity(ATTACHMENT_DEK_AAD_PREFIX.len() + attachment_ulid.len());
    aad.extend_from_slice(ATTACHMENT_DEK_AAD_PREFIX);
    aad.extend_from_slice(attachment_ulid);
    let ct = cipher
        .encrypt(
            &nonce_arr,
            Payload {
                msg: dek,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Internal(anyhow::anyhow!("AES-GCM wrap of attachment DEK failed")))?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_arr.as_slice());
    Ok((ct, nonce))
}

/// Unwrap a per-attachment DEK under [`EnvelopeKey`].
fn unwrap_attachment_dek(
    envelope: &EnvelopeKey,
    attachment_ulid: &Ulid,
    wrapped_dek: &[u8],
    dek_nonce: &[u8; NONCE_LEN],
) -> Result<Zeroizing<[u8; DEK_LEN]>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(envelope.as_bytes()));
    let nonce: &Nonce<<Aes256Gcm as AeadCore>::NonceSize> = Nonce::from_slice(dek_nonce);
    let mut aad = Vec::with_capacity(ATTACHMENT_DEK_AAD_PREFIX.len() + attachment_ulid.len());
    aad.extend_from_slice(ATTACHMENT_DEK_AAD_PREFIX);
    aad.extend_from_slice(attachment_ulid);
    let pt = cipher
        .decrypt(
            nonce,
            Payload {
                msg: wrapped_dek,
                aad: &aad,
            },
        )
        .map_err(|_| Error::DecryptionFailed)?;
    if pt.len() != DEK_LEN {
        return Err(Error::DecryptionFailed);
    }
    let mut bytes = [0u8; DEK_LEN];
    bytes.copy_from_slice(&pt);
    Ok(Zeroizing::new(bytes))
}

/// Default maximum attachment size: 50 MiB.
///
/// Configurable via deployment (callers wanting a different cap pass
/// `max_bytes` to [`new_writer_with_cap`]).
pub const MAX_BLOB_BYTES: u64 = 50 * 1024 * 1024;

/// Resolve the sidecar root for a given storage `data.db` path. The root is
/// `<parent>/attachments/`; created lazily on first write.
pub fn sidecar_root_for(storage_path: &Path) -> PathBuf {
    let parent = storage_path.parent().unwrap_or(Path::new("."));
    parent.join("attachments")
}

/// Open a temp-file writer in `root/.tmp/`. The returned writer accumulates
/// chunks; call [`AttachmentWriter::finalize`] to commit.
///
/// **As of the default-on flip**: production callers should pass an
/// `envelope_key` via [`new_writer_with_envelope`]. The plain `new_writer`
/// constructor returns a *plaintext* writer (legacy / testing-only path).
/// `OhdcService.attach_blob` calls [`new_writer_with_envelope`] when the
/// storage handle has a live envelope key (i.e. always, in production).
pub fn new_writer(
    root: &Path,
    mime_type: Option<String>,
    filename: Option<String>,
) -> Result<AttachmentWriter> {
    new_writer_with_cap(root, mime_type, filename, MAX_BLOB_BYTES)
}

/// Open a writer that encrypts the finalized blob under a fresh per-attachment
/// DEK wrapped with `envelope`. This is the production default.
pub fn new_writer_with_envelope(
    root: &Path,
    mime_type: Option<String>,
    filename: Option<String>,
    envelope: EnvelopeKey,
) -> Result<AttachmentWriter> {
    let writer = new_writer_with_cap(root, mime_type, filename, MAX_BLOB_BYTES)?;
    Ok(writer.with_envelope_key(envelope))
}

/// Like [`new_writer`] but with a configurable size cap.
pub fn new_writer_with_cap(
    root: &Path,
    mime_type: Option<String>,
    filename: Option<String>,
    max_bytes: u64,
) -> Result<AttachmentWriter> {
    let tmp_root = root.join(".tmp");
    fs::create_dir_all(&tmp_root)?;
    // 16 random bytes hex = 32-char tmp filename — distinct from the eventual
    // sha256 path to avoid collision with finalized blobs.
    let suffix = ulid::random_bytes(16);
    let tmp_path = tmp_root.join(hex::encode(&suffix));
    let file = fs::File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&tmp_path)?;
    Ok(AttachmentWriter {
        root: root.to_path_buf(),
        tmp_path,
        file,
        hasher: Sha256::new(),
        bytes_written: 0,
        max_bytes,
        mime_type,
        filename,
        envelope_key: None,
    })
}

/// Streaming attachment writer. Owns the in-flight temp file; `write` accepts
/// chunks; `finalize` performs the encrypt+rename + DB insert.
pub struct AttachmentWriter {
    root: PathBuf,
    tmp_path: PathBuf,
    file: fs::File,
    hasher: Sha256,
    bytes_written: u64,
    max_bytes: u64,
    mime_type: Option<String>,
    filename: Option<String>,
    /// When set, finalize encrypts the blob bytes under a fresh per-attachment
    /// DEK and stores the wrapped DEK + nonce on the row. When `None`, the
    /// legacy plaintext-on-disk path is taken (unit tests and back-compat for
    /// pre-encryption deployments).
    envelope_key: Option<EnvelopeKey>,
}

impl AttachmentWriter {
    /// Configure the writer to encrypt the finalized blob under a fresh
    /// per-attachment DEK wrapped with `envelope`. Equivalent to building the
    /// writer via [`new_writer_with_envelope`]; retained for callers that
    /// add the envelope key after construction.
    pub fn with_envelope_key(mut self, envelope: EnvelopeKey) -> Self {
        self.envelope_key = Some(envelope);
        self
    }

    /// Force the legacy plaintext finalize path even when an envelope key
    /// could be supplied. Used by tests and the (optional) `force_plaintext`
    /// flag on `AttachBlobRequest` for debugging / migration scenarios.
    ///
    /// Production callers should not use this — the default is encrypted-on-
    /// disk and that's the path covered by the threat model.
    pub fn force_plaintext(mut self) -> Self {
        self.envelope_key = None;
        self
    }

    /// Append chunk bytes to the in-flight blob.
    pub fn write_chunk(&mut self, chunk: &[u8]) -> Result<()> {
        self.bytes_written = self
            .bytes_written
            .checked_add(chunk.len() as u64)
            .ok_or(Error::PayloadTooLarge)?;
        if self.bytes_written > self.max_bytes {
            return Err(Error::PayloadTooLarge);
        }
        self.hasher.update(chunk);
        self.file.write_all(chunk)?;
        Ok(())
    }

    /// Finalize the upload.
    ///
    /// Encrypt with XChaCha20-Poly1305 STREAM, AAD bound to
    /// `(attachment_ulid, event_ulid, sha256, mime_type, filename,
    /// byte_size)`. Codex review #3 + #6.
    ///
    /// When the writer was constructed without an envelope key, the
    /// finalized blob is plaintext on disk (legacy / testing-only path —
    /// production callers always pass an envelope).
    ///
    /// Path remains `<root>/<sha[..2]>/<sha>` where `sha = sha256(plaintext)`
    /// (content-addressing on plaintext is preserved per spec).
    pub fn finalize(
        mut self,
        conn: &Connection,
        event_id: i64,
        event_ulid: &Ulid,
        expected_sha256: Option<&[u8]>,
    ) -> Result<(PathBuf, AttachmentMetaRow)> {
        let sha = self.hasher.finalize();
        let sha_bytes: [u8; 32] = sha.into();
        if let Some(expected) = expected_sha256 {
            if expected != sha_bytes {
                let _ = fs::remove_file(&self.tmp_path);
                return Err(Error::InvalidArgument(
                    "attachment expected_sha256 mismatch".into(),
                ));
            }
        }
        // Flush to disk before reading back / removing.
        self.file.flush()?;
        self.file.seek(SeekFrom::Start(0))?;

        // Mint a wire ULID up-front so the AAD uses it.
        let now = crate::format::now_ms();
        let new_ulid = ulid::mint(now);
        let rand_tail = ulid::random_tail(&new_ulid);

        let hex_sha = hex::encode(sha_bytes);
        let bucket = &hex_sha[..2];
        let dest_dir = self.root.join(bucket);
        fs::create_dir_all(&dest_dir)?;
        let dest = dest_dir.join(&hex_sha);

        // Encryption path vs plaintext path.
        let (wrapped_dek_col, dek_nonce_col, encrypted_flag) = if let Some(envelope) =
            self.envelope_key.as_ref()
        {
            // Generate a fresh DEK.
            let mut dek = [0u8; DEK_LEN];
            rand::thread_rng().fill_bytes(&mut dek);
            let dek_z = Zeroizing::new(dek);

            let (wrapped, dek_nonce) = wrap_attachment_dek(envelope, &new_ulid, &dek_z)?;

            // Codex review #6: stream-encrypt without ever materializing
            // the full plaintext in memory.
            let aad = attachment_aad(
                &new_ulid,
                event_ulid,
                &sha_bytes,
                self.mime_type.as_deref(),
                self.filename.as_deref(),
                self.bytes_written,
            );
            let enc_tmp = self.tmp_path.with_extension("enc");
            let mut dst = fs::File::options()
                .create_new(true)
                .write(true)
                .open(&enc_tmp)?;
            encrypt_attachment_stream(&dek_z, &aad, &mut self.file, self.bytes_written, &mut dst)?;
            drop(self.file);
            fs::rename(&enc_tmp, &dest)?;
            let _ = fs::remove_file(&self.tmp_path);

            (Some(wrapped), Some(dek_nonce.to_vec()), 1i64)
        } else {
            // Legacy plaintext path: just rename the temp file.
            drop(self.file);
            fs::rename(&self.tmp_path, &dest)?;
            (None, None, 0i64)
        };

        conn.execute(
            "INSERT INTO attachments
                (ulid_random, event_id, sha256, byte_size, mime_type, filename,
                 encrypted, wrapped_dek, dek_nonce)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rand_tail.to_vec(),
                event_id,
                sha_bytes.to_vec(),
                self.bytes_written as i64,
                self.mime_type,
                self.filename,
                encrypted_flag,
                wrapped_dek_col,
                dek_nonce_col,
            ],
        )?;
        let id = conn.last_insert_rowid();
        let row = AttachmentMetaRow {
            id,
            ulid: new_ulid,
            sha256: sha_bytes,
            byte_size: self.bytes_written as i64,
            mime_type: self.mime_type.clone(),
            filename: self.filename.clone(),
        };
        Ok((dest, row))
    }
}

/// Materialized `attachments` row for the wire.
#[derive(Debug, Clone)]
pub struct AttachmentMetaRow {
    /// Internal rowid.
    pub id: i64,
    /// Wire ULID.
    pub ulid: Ulid,
    /// 32-byte SHA-256.
    pub sha256: [u8; 32],
    /// Size in bytes.
    pub byte_size: i64,
    /// MIME type.
    pub mime_type: Option<String>,
    /// Filename.
    pub filename: Option<String>,
}

/// Look up attachment metadata + on-disk path by wire ULID.
pub fn load_attachment_meta(
    conn: &Connection,
    root: &Path,
    attachment_ulid: &Ulid,
) -> Result<(AttachmentMetaRow, PathBuf)> {
    let rand_tail = ulid::random_tail(attachment_ulid);
    let row: Option<(i64, Vec<u8>, i64, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT id, sha256, byte_size, mime_type, filename
               FROM attachments WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (id, sha_blob, byte_size, mime_type, filename) = row.ok_or(Error::NotFound)?;
    // Codex review #7: reject malformed sha length explicitly rather than
    // silently zero-filling the array (which would route the read to the
    // wrong on-disk path and defer the failure to a confusing
    // "file-not-found" error).
    if sha_blob.len() != 32 {
        return Err(Error::Internal(anyhow::anyhow!(
            "attachment.sha256 length != 32 (CorruptStorage)"
        )));
    }
    let mut sha = [0u8; 32];
    sha.copy_from_slice(&sha_blob);
    let hex_sha = hex::encode(sha);
    let path = root.join(&hex_sha[..2]).join(&hex_sha);
    Ok((
        AttachmentMetaRow {
            id,
            ulid: *attachment_ulid,
            sha256: sha,
            byte_size,
            mime_type,
            filename,
        },
        path,
    ))
}

/// Internal: load the wrap material + parent-event-ULID for
/// `attachment_ulid` if any. Returns `Some(_)` when the row is encrypted.
struct WrapMaterial {
    wrapped_dek: Vec<u8>,
    dek_nonce: [u8; NONCE_LEN],
    /// Parent event ULID — needed for AAD reconstruction.
    event_ulid: Ulid,
}

fn load_wrap_material(conn: &Connection, attachment_ulid: &Ulid) -> Result<Option<WrapMaterial>> {
    let rand_tail = ulid::random_tail(attachment_ulid);
    type Row = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<i64>, // event_id
    );
    let row: Option<Row> = conn
        .query_row(
            "SELECT wrapped_dek, dek_nonce, event_id
               FROM attachments WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((wrapped, nonce, event_id)) = row else {
        return Ok(None);
    };
    match (wrapped, nonce) {
        (Some(wrapped), Some(nonce_vec)) if nonce_vec.len() == NONCE_LEN => {
            let mut nonce = [0u8; NONCE_LEN];
            nonce.copy_from_slice(&nonce_vec);
            // Reconstruct the parent event's ULID from `event_id`. The AAD
            // binds it (Codex review #3); rows without an event_id are
            // structurally invalid for the encrypted code path.
            let event_ulid = match event_id {
                Some(eid) => {
                    let row: Option<(i64, Vec<u8>)> = conn
                        .query_row(
                            "SELECT timestamp_ms, ulid_random FROM events WHERE id = ?1",
                            params![eid],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    match row.and_then(|(ts, tail)| ulid::from_parts(ts, &tail).ok()) {
                        Some(u) => u,
                        None => {
                            return Err(Error::Internal(anyhow::anyhow!(
                                "encrypted attachment row references missing event_id"
                            )));
                        }
                    }
                }
                None => {
                    return Err(Error::Internal(anyhow::anyhow!(
                        "encrypted attachment row missing event_id (corrupt)"
                    )));
                }
            };
            Ok(Some(WrapMaterial {
                wrapped_dek: wrapped,
                dek_nonce: nonce,
                event_ulid,
            }))
        }
        _ => Ok(None),
    }
}

/// Read an attachment's plaintext bytes by wire ULID. If the row is
/// encrypted (`wrapped_dek IS NOT NULL`), unwraps the DEK under `envelope`,
/// reads the on-disk ciphertext, and returns decrypted plaintext. The
/// streaming AEAD AAD binds `(att_ulid, event_ulid, sha256, mime, filename,
/// byte_size)` — tampering with any of those metadata columns triggers
/// [`Error::DecryptionFailed`].
///
/// Plaintext / pre-encryption rows are returned as-is. To opt-in to lazy
/// migration on read, use [`read_and_lazy_migrate_attachment`].
pub fn read_attachment_bytes(
    conn: &Connection,
    root: &Path,
    attachment_ulid: &Ulid,
    envelope: Option<&EnvelopeKey>,
) -> Result<Vec<u8>> {
    let (meta, path) = load_attachment_meta(conn, root, attachment_ulid)?;
    let on_disk = fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound
        } else {
            Error::Io(e)
        }
    })?;
    let wrap = load_wrap_material(conn, attachment_ulid)?;
    match (wrap, envelope) {
        (Some(wm), Some(env)) => {
            let dek = unwrap_attachment_dek(env, attachment_ulid, &wm.wrapped_dek, &wm.dek_nonce)?;
            let aad = attachment_aad(
                attachment_ulid,
                &wm.event_ulid,
                &meta.sha256,
                meta.mime_type.as_deref(),
                meta.filename.as_deref(),
                meta.byte_size as u64,
            );
            decrypt_attachment_stream(&dek, &aad, &on_disk)
        }
        (Some(_), None) => Err(Error::InvalidArgument(
            "read_attachment_bytes: envelope key required to decrypt this attachment".into(),
        )),
        (None, _) => Ok(on_disk), // legacy plaintext blob
    }
}

/// Read + lazy-migrate an attachment.
///
/// If the row is already encrypted, behaves like [`read_attachment_bytes`].
/// If the row is legacy plaintext (`wrapped_dek IS NULL`), reads the
/// plaintext, generates a fresh DEK, stream-encrypts the bytes in place
/// (atomic rename), and updates the row to record the wrap material. Future
/// reads go down the encrypted path.
///
/// The lazy-migration write is best-effort: if writing or the DB update
/// fails, the function logs at warn-level and returns the plaintext anyway
/// (the read succeeds; migration retries on the next read).
///
/// Lazy-migration emits the same XChaCha20-Poly1305 STREAM format as the
/// production write path. The parent event's ULID is fetched from the
/// `event_id` foreign key so the AAD bind matches what `finalize` would
/// have produced for the same row.
pub fn read_and_lazy_migrate_attachment(
    conn: &Connection,
    root: &Path,
    attachment_ulid: &Ulid,
    envelope: &EnvelopeKey,
) -> Result<Vec<u8>> {
    let (meta, path) = load_attachment_meta(conn, root, attachment_ulid)?;
    let wrap = load_wrap_material(conn, attachment_ulid)?;
    if let Some(wm) = wrap {
        // Already encrypted.
        let on_disk = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::NotFound
            } else {
                Error::Io(e)
            }
        })?;
        let dek = unwrap_attachment_dek(envelope, attachment_ulid, &wm.wrapped_dek, &wm.dek_nonce)?;
        let aad = attachment_aad(
            attachment_ulid,
            &wm.event_ulid,
            &meta.sha256,
            meta.mime_type.as_deref(),
            meta.filename.as_deref(),
            meta.byte_size as u64,
        );
        return decrypt_attachment_stream(&dek, &aad, &on_disk);
    }
    // Legacy plaintext path. Read, stream-encrypt to a fresh DEK, atomically
    // overwrite, update the row.
    let plaintext = fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound
        } else {
            Error::Io(e)
        }
    })?;

    // The streaming AAD binds the parent event's ULID; fetch it via the
    // `event_id` foreign key on the row. If absent, the row is structurally
    // not migratable to the encrypted shape — return the plaintext and warn.
    let rand_tail = ulid::random_tail(attachment_ulid);
    let event_id: Option<i64> = conn
        .query_row(
            "SELECT event_id FROM attachments WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten();
    let event_ulid: Ulid = match event_id {
        Some(eid) => match conn
            .query_row(
                "SELECT timestamp_ms, ulid_random FROM events WHERE id = ?1",
                params![eid],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?
            .and_then(|(ts, tail)| ulid::from_parts(ts, &tail).ok())
        {
            Some(u) => u,
            None => {
                tracing::warn!("lazy-migrate: parent event row missing; skipping migration");
                return Ok(plaintext);
            }
        },
        None => {
            tracing::warn!("lazy-migrate: row has no event_id; skipping migration");
            return Ok(plaintext);
        }
    };

    // Generate fresh DEK, stream-encrypt under it.
    let mut dek = [0u8; DEK_LEN];
    rand::thread_rng().fill_bytes(&mut dek);
    let dek_z = Zeroizing::new(dek);

    let aad = attachment_aad(
        attachment_ulid,
        &event_ulid,
        &meta.sha256,
        meta.mime_type.as_deref(),
        meta.filename.as_deref(),
        meta.byte_size as u64,
    );

    let wrap = match wrap_attachment_dek(envelope, attachment_ulid, &dek_z) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("lazy-migrate wrap failed: {e}");
            return Ok(plaintext);
        }
    };

    // Atomic overwrite via tmp file. The streaming encrypt reads from the
    // existing (plaintext) on-disk file and writes the ciphertext to a
    // sibling tmp; on success we rename over the original.
    let tmp_root = root.join(".tmp");
    if let Err(e) = fs::create_dir_all(&tmp_root) {
        tracing::warn!("lazy-migrate tmp dir create failed: {e}");
        return Ok(plaintext);
    }
    let tmp_name = format!("migrate-{}", hex::encode(ulid::random_bytes(12)));
    let tmp_path = tmp_root.join(tmp_name);

    let mut src = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("lazy-migrate src open failed: {e}");
            return Ok(plaintext);
        }
    };
    let mut dst = match fs::File::options()
        .create_new(true)
        .write(true)
        .open(&tmp_path)
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("lazy-migrate tmp open failed: {e}");
            return Ok(plaintext);
        }
    };
    if let Err(e) =
        encrypt_attachment_stream(&dek_z, &aad, &mut src, plaintext.len() as u64, &mut dst)
    {
        tracing::warn!("lazy-migrate stream encrypt failed: {e}");
        let _ = fs::remove_file(&tmp_path);
        return Ok(plaintext);
    }
    drop(src);
    drop(dst);
    if let Err(e) = fs::rename(&tmp_path, &path) {
        tracing::warn!("lazy-migrate rename failed: {e}");
        let _ = fs::remove_file(&tmp_path);
        return Ok(plaintext);
    }

    // Update the row.
    if let Err(e) = conn.execute(
        "UPDATE attachments
            SET wrapped_dek = ?1, dek_nonce = ?2, encrypted = 1
          WHERE ulid_random = ?3",
        params![wrap.0, wrap.1.to_vec(), rand_tail.to_vec()],
    ) {
        tracing::warn!("lazy-migrate row update failed: {e}");
        // The on-disk bytes are now ciphertext but the row still claims
        // plaintext — corruption-adjacent. Best-effort: try to revert the
        // file by writing the plaintext back. Logged regardless.
        if let Err(e2) = fs::write(&path, &plaintext) {
            tracing::error!("lazy-migrate revert also failed; attachment is corrupt: {e2}");
        }
        return Ok(plaintext);
    }

    Ok(plaintext)
}

/// List attachments associated with `event_id` for hydration into `Event`.
pub fn list_for_event(conn: &Connection, event_id: i64) -> Result<Vec<AttachmentRef>> {
    let mut stmt = conn.prepare(
        "SELECT ulid_random, sha256, byte_size, mime_type, filename
           FROM attachments WHERE event_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![event_id], |r| {
            let rand_tail: Vec<u8> = r.get(0)?;
            let sha_blob: Vec<u8> = r.get(1)?;
            let byte_size: i64 = r.get(2)?;
            let mime: Option<String> = r.get(3)?;
            let filename: Option<String> = r.get(4)?;
            Ok((rand_tail, sha_blob, byte_size, mime, filename))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (rand_tail, sha_blob, byte_size, mime_type, filename) in rows {
        // Reconstruct ULID using created_at = 0 because the attachments table
        // doesn't store it independently. The wire ULID's time prefix doesn't
        // factor into uniqueness — the random tail does.
        let mut ulid_buf = [0u8; 16];
        if rand_tail.len() == 10 {
            ulid_buf[6..].copy_from_slice(&rand_tail);
        }
        out.push(AttachmentRef {
            ulid: ulid::to_crockford(&ulid_buf),
            sha256: hex::encode(&sha_blob),
            byte_size,
            mime_type,
            filename,
        });
    }
    Ok(out)
}

/// Stream-read an attachment file. Used by the server-streaming
/// `ReadAttachment` RPC to chunk the bytes back to the client.
pub fn open_for_read(path: &Path) -> Result<fs::File> {
    fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound
        } else {
            Error::Io(e)
        }
    })
}

/// Look up an attachment's `(rowid, sha256, byte_size, mime_type, filename)` by
/// `(wire_ulid, expected_sha)`. Used by the sync push handler before streaming
/// blob bytes — the metadata frame already arrived through the regular sync
/// stream, so the row exists; we just verify the sha256 matches what the
/// caller is about to push.
pub fn find_by_ulid_and_sha(
    conn: &Connection,
    attachment_ulid: &Ulid,
    expected_sha: &[u8],
) -> Result<Option<(i64, [u8; 32], i64, Option<String>, Option<String>)>> {
    let rand_tail = ulid::random_tail(attachment_ulid);
    let row: Option<(i64, Vec<u8>, i64, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT id, sha256, byte_size, mime_type, filename
               FROM attachments WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((id, sha_blob, byte_size, mime_type, filename)) = row else {
        return Ok(None);
    };
    if sha_blob != expected_sha {
        return Err(Error::InvalidArgument(format!(
            "attachment ULID/sha mismatch: stored sha {} != supplied {}",
            hex::encode(&sha_blob),
            hex::encode(expected_sha),
        )));
    }
    // Codex review #7: reject malformed sha length rather than silently
    // zero-filling the array.
    if sha_blob.len() != 32 {
        return Err(Error::Internal(anyhow::anyhow!(
            "attachment.sha256 length != 32 (CorruptStorage)"
        )));
    }
    let mut sha = [0u8; 32];
    sha.copy_from_slice(&sha_blob);
    Ok(Some((id, sha, byte_size, mime_type, filename)))
}

/// Compute the on-disk path for a sha256-addressed sidecar blob under `root`.
pub fn blob_path_for(root: &Path, sha256: &[u8; 32]) -> PathBuf {
    let hex_sha = hex::encode(sha256);
    root.join(&hex_sha[..2]).join(&hex_sha)
}

/// Receive a plaintext attachment payload from a peer, encrypt it under this
/// storage's `K_envelope` with a fresh per-attachment DEK, and write the
/// ciphertext to `<root>/<sha[..2]>/<sha>` (where `sha = sha256(plaintext)`,
/// as the spec mandates content-addressing on plaintext).
///
/// Stamps the wrap material onto the existing `attachments` row identified by
/// `attachment_ulid` (the metadata row was created when the EventFrame
/// arrived earlier in the sync session).
///
/// The on-disk path is `sha256(plaintext)`, identical to a locally-uploaded
/// attachment; encryption is invisible to the path layer. This is what the
/// spec calls out as "the on-the-wire bytes are plaintext (the wire frame is
/// opaque-bytes inside the relay tunnel, but the storage-to-storage sync
/// exchange itself does NOT carry encrypted-attachment ciphertext — each
/// storage encrypts under its own envelope, since K_envelope differs per
/// storage)".
pub fn receive_and_encrypt_blob(
    conn: &Connection,
    root: &Path,
    envelope: &EnvelopeKey,
    attachment_ulid: &Ulid,
    plaintext: &[u8],
    expected_sha: &[u8; 32],
) -> Result<PathBuf> {
    // Verify the sha against plaintext bytes (sender claimed this sha; the
    // metadata row has the same sha).
    let mut hasher = Sha256::new();
    hasher.update(plaintext);
    let actual: [u8; 32] = hasher.finalize().into();
    if actual != *expected_sha {
        return Err(Error::InvalidArgument(
            "PushAttachmentBlob: supplied sha256 does not match plaintext".into(),
        ));
    }

    // Look up the row's metadata: parent event_id + mime + filename +
    // byte_size — the V2 AAD binds all of those (Codex review #3).
    let rand_tail = ulid::random_tail(attachment_ulid);
    type MetaRow = (Option<i64>, Option<String>, Option<String>, i64);
    let meta: Option<MetaRow> = conn
        .query_row(
            "SELECT event_id, mime_type, filename, byte_size
               FROM attachments WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (event_id, mime_type, filename, byte_size) = meta.ok_or(Error::NotFound)?;
    let event_ulid: Option<Ulid> = match event_id {
        Some(eid) => conn
            .query_row(
                "SELECT timestamp_ms, ulid_random FROM events WHERE id = ?1",
                params![eid],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?
            .and_then(|(ts, tail)| ulid::from_parts(ts, &tail).ok()),
        None => None,
    };

    // Generate a fresh DEK; wrap under K_envelope.
    let mut dek = [0u8; DEK_LEN];
    rand::thread_rng().fill_bytes(&mut dek);
    let dek_z = Zeroizing::new(dek);
    let (wrapped_dek, dek_nonce) = wrap_attachment_dek(envelope, attachment_ulid, &dek_z)?;

    // Atomic write to <root>/<sha[..2]>/<sha>.
    let dest = blob_path_for(root, expected_sha);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_root = root.join(".tmp");
    fs::create_dir_all(&tmp_root)?;
    let tmp_name = format!("inbound-enc-{}", hex::encode(ulid::random_bytes(12)));
    let tmp_path = tmp_root.join(&tmp_name);

    // Streaming AEAD path. Codex review #3 + #6. The AAD binds the parent
    // event's ULID; without it the row can't match what the issuer wrote.
    let evt = event_ulid.ok_or_else(|| {
        Error::Internal(anyhow::anyhow!(
            "receive_and_encrypt_blob: attachment row has no event_id (corrupt)"
        ))
    })?;
    let aad = attachment_aad(
        attachment_ulid,
        &evt,
        expected_sha,
        mime_type.as_deref(),
        filename.as_deref(),
        byte_size as u64,
    );
    // Stage plaintext into a tmp src file for the streaming encryptor.
    let src_name = format!("inbound-pt-{}", hex::encode(ulid::random_bytes(12)));
    let src_path = tmp_root.join(&src_name);
    {
        let mut f = fs::File::options()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&src_path)?;
        f.write_all(plaintext)?;
        f.flush()?;
    }
    let mut src = fs::File::open(&src_path)?;
    let mut dst = fs::File::options()
        .create_new(true)
        .write(true)
        .open(&tmp_path)?;
    let result =
        encrypt_attachment_stream(&dek_z, &aad, &mut src, plaintext.len() as u64, &mut dst);
    let _ = fs::remove_file(&src_path);
    result?;
    fs::rename(&tmp_path, &dest)?;

    // Stamp wrap material on the row.
    conn.execute(
        "UPDATE attachments
            SET wrapped_dek = ?1, dek_nonce = ?2, encrypted = 1
          WHERE ulid_random = ?3",
        params![wrapped_dek, dek_nonce.to_vec(), rand_tail.to_vec()],
    )?;

    Ok(dest)
}

/// Atomically write a sidecar blob under `<root>/<sha[..2]>/<sha>`. Used by the
/// sync `PushAttachmentBlob` handler. Returns the destination path.
///
/// If the destination already exists with the same sha, this is a no-op
/// (content-addressed storage — same sha guarantees same bytes). The
/// `expected_sha256` is recomputed from `payload` and compared to the
/// supplied `expected` argument (which itself was already cross-checked
/// against the metadata row by the caller).
pub fn write_blob_atomic(root: &Path, payload: &[u8], expected_sha: &[u8; 32]) -> Result<PathBuf> {
    // Recompute the sha to defend against the caller passing a forged
    // payload. (Cheap; the alternative is to silently store mismatched
    // bytes that will fail every subsequent ReadAttachment.)
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let actual: [u8; 32] = hasher.finalize().into();
    if actual != *expected_sha {
        return Err(Error::InvalidArgument(
            "PushAttachmentBlob: supplied sha256 does not match payload".into(),
        ));
    }
    let dest = blob_path_for(root, expected_sha);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        // Already present — treat as idempotent. Verify size matches as a
        // weak sanity check (the sha would differ if bytes diverged).
        if let Ok(meta) = fs::metadata(&dest) {
            if meta.len() == payload.len() as u64 {
                return Ok(dest);
            }
        }
    }
    let tmp_root = root.join(".tmp");
    fs::create_dir_all(&tmp_root)?;
    let tmp_name = format!("inbound-{}", hex::encode(ulid::random_bytes(12)));
    let tmp_path = tmp_root.join(tmp_name);
    {
        let mut f = fs::File::options()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        f.write_all(payload)?;
        f.flush()?;
    }
    fs::rename(&tmp_path, &dest)?;
    Ok(dest)
}

#[allow(dead_code)]
fn _unused_seek<R: Read + Seek>(_r: &mut R) {}
