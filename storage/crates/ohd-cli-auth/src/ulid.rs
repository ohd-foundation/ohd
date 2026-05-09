//! Crockford-base32 ULID display + parse helpers.
//!
//! OHDC's wire format is the canonical 16-byte big-endian binary; humans
//! see the 26-char Crockford base32 form. Mirror of
//! `ohd_storage_core::ulid` `to_crockford` / `parse_crockford` — kept as
//! a tiny copy here so the CLIs don't need to take the heavy storage-core
//! crate (with its SQLite + AEAD + BIP39 deps) just to render a ULID.

use anyhow::{anyhow, Result};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode 16 ULID bytes to the 26-char Crockford base32 string.
pub fn to_crockford(ulid: &[u8; 16]) -> String {
    let mut acc: u128 = 0;
    for &b in ulid {
        acc = (acc << 8) | b as u128;
    }
    let mut out = [0u8; 26];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = 5 * (25 - i);
        let idx = ((acc >> shift) & 0x1f) as usize;
        *slot = CROCKFORD[idx];
    }
    String::from_utf8(out.to_vec()).expect("ascii")
}

/// Best-effort: render whatever bytes are in a `pb::Ulid` field. Wire spec
/// says exactly 16; if storage ever returned shorter we want to see that
/// rather than panic.
pub fn render_ulid_bytes(bytes: &[u8]) -> String {
    if bytes.len() == 16 {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        to_crockford(&arr)
    } else {
        format!("<malformed ULID: {} bytes>", bytes.len())
    }
}

/// Parse a 26-char Crockford-base32 ULID into its 16-byte form.
///
/// Crockford base32 is case-insensitive and ignores ambiguous chars
/// (`I`/`L` map to `1`, `O` maps to `0`). We follow the same convention.
pub fn parse_crockford(s: &str) -> Result<[u8; 16]> {
    if s.len() != 26 {
        return Err(anyhow!(
            "ULID must be 26 Crockford-base32 chars, got {}",
            s.len()
        ));
    }
    let mut acc: u128 = 0;
    for (i, ch) in s.chars().enumerate() {
        let v: u128 = match ch.to_ascii_uppercase() {
            '0' | 'O' => 0,
            '1' | 'I' | 'L' => 1,
            '2' => 2,
            '3' => 3,
            '4' => 4,
            '5' => 5,
            '6' => 6,
            '7' => 7,
            '8' => 8,
            '9' => 9,
            'A' => 10,
            'B' => 11,
            'C' => 12,
            'D' => 13,
            'E' => 14,
            'F' => 15,
            'G' => 16,
            'H' => 17,
            'J' => 18,
            'K' => 19,
            'M' => 20,
            'N' => 21,
            'P' => 22,
            'Q' => 23,
            'R' => 24,
            'S' => 25,
            'T' => 26,
            'V' => 27,
            'W' => 28,
            'X' => 29,
            'Y' => 30,
            'Z' => 31,
            other => {
                return Err(anyhow!(
                    "non-Crockford-base32 char {other:?} at position {i}"
                ));
            }
        };
        acc = (acc << 5) | v;
    }
    Ok(acc.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let bytes: [u8; 16] = [
            0x01, 0x8E, 0xC9, 0x5B, 0xA9, 0x2D, 0x6B, 0x8C, 0x77, 0x12, 0x34, 0x56, 0x78, 0x9A,
            0xBC, 0xDE,
        ];
        let s = to_crockford(&bytes);
        assert_eq!(s.len(), 26);
        let back = parse_crockford(&s).unwrap();
        assert_eq!(back, bytes);
    }

    #[test]
    fn lower_case_and_aliases() {
        let bytes = [0u8; 16];
        let s = to_crockford(&bytes);
        let lower = s.to_lowercase();
        assert_eq!(parse_crockford(&lower).unwrap(), bytes);
        // O -> 0, I/L -> 1 — substituting these inside an all-zeros ULID
        // yields a different valid ULID, which is the behaviour callers
        // want to see (forgiving paste from copy-pasted IDs).
        assert!(parse_crockford("OOOOOOOOOOOOOOOOOOOOOOOOOO").is_ok());
        assert!(parse_crockford("IIIIIIIIIIIIIIIIIIIIIIIIII").is_ok());
        assert!(parse_crockford("LLLLLLLLLLLLLLLLLLLLLLLLLL").is_ok());
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(parse_crockford("ABCDE").is_err());
    }

    #[test]
    fn rejects_invalid_char() {
        let mut s = "0".repeat(26);
        // 'U' is excluded from Crockford base32 (ambiguity with V).
        s.replace_range(0..1, "U");
        assert!(parse_crockford(&s).is_err());
    }
}
