//! ULID minting + Crockford-base32 encoding.
//!
//! Wire form: 16 bytes big-endian. The 48-bit time prefix equals
//! `timestamp_ms` clamped to `[0, 2^48-1]`; for events with `timestamp_ms < 0`
//! the prefix is clamped to 0 and uniqueness is carried by the 80-bit random
//! tail (covered by `idx_events_ulid`).
//!
//! Display form is the 26-char Crockford base32 string per the ULID spec.

use rand::RngCore;

use crate::{Error, Result};

/// 16-byte ULID — wire identity for events, attachments, grants, cases, etc.
pub type Ulid = [u8; 16];

/// Mint a fresh ULID for a given measurement timestamp.
///
/// - If `timestamp_ms >= 0`: time-prefix is `timestamp_ms` truncated to 48 bits.
/// - If `timestamp_ms < 0`: time-prefix is clamped to `0`.
///
/// The 80-bit random tail is filled from the system CSPRNG.
pub fn mint(timestamp_ms: i64) -> Ulid {
    let mut buf = [0u8; 16];
    let ts = if timestamp_ms < 0 {
        0u64
    } else {
        timestamp_ms as u64
    };
    let ts48 = ts & 0x0000_FFFF_FFFF_FFFF;
    buf[0] = ((ts48 >> 40) & 0xff) as u8;
    buf[1] = ((ts48 >> 32) & 0xff) as u8;
    buf[2] = ((ts48 >> 24) & 0xff) as u8;
    buf[3] = ((ts48 >> 16) & 0xff) as u8;
    buf[4] = ((ts48 >> 8) & 0xff) as u8;
    buf[5] = (ts48 & 0xff) as u8;
    rand::thread_rng().fill_bytes(&mut buf[6..16]);
    buf
}

/// Random byte buffer of length `n`.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut v);
    v
}

/// Just the random tail (10 bytes), what `events.ulid_random` stores.
pub fn random_tail(ulid: &Ulid) -> [u8; 10] {
    let mut out = [0u8; 10];
    out.copy_from_slice(&ulid[6..16]);
    out
}

/// Reassemble a ULID from `(timestamp_ms, ulid_random)`.
pub fn from_parts(timestamp_ms: i64, random: &[u8]) -> Result<Ulid> {
    if random.len() != 10 {
        return Err(Error::InvalidUlid);
    }
    let mut out = mint(timestamp_ms);
    out[6..16].copy_from_slice(random);
    Ok(out)
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode to 26-char Crockford base32 (display form).
pub fn to_crockford(ulid: &Ulid) -> String {
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

/// Parse a 26-char Crockford base32 string.
pub fn parse_crockford(s: &str) -> Result<Ulid> {
    let s = s.trim();
    if s.len() != 26 {
        return Err(Error::InvalidUlid);
    }
    let mut acc: u128 = 0;
    for c in s.bytes() {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'H' => c - b'A' + 10,
            b'J' => 18,
            b'K' => 19,
            b'M' => 20,
            b'N' => 21,
            b'P' => 22,
            b'Q' => 23,
            b'R' => 24,
            b'S' => 25,
            b'T' => 26,
            b'V' => 27,
            b'W' => 28,
            b'X' => 29,
            b'Y' => 30,
            b'Z' => 31,
            b'a'..=b'h' => c - b'a' + 10,
            b'j' => 18,
            b'k' => 19,
            b'm' => 20,
            b'n' => 21,
            b'p' => 22,
            b'q' => 23,
            b'r' => 24,
            b's' => 25,
            b't' => 26,
            b'v' => 27,
            b'w' => 28,
            b'x' => 29,
            b'y' => 30,
            b'z' => 31,
            _ => return Err(Error::InvalidUlid),
        };
        acc = (acc << 5) | v as u128;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[15 - i] = ((acc >> (8 * i)) & 0xff) as u8;
    }
    Ok(out)
}

/// Split a 16-byte ULID into `(time_prefix_ms, random_80bit)`.
pub fn split(u: &Ulid) -> (i64, [u8; 10]) {
    let mut ts: u64 = 0;
    for &b in &u[..6] {
        ts = (ts << 8) | b as u64;
    }
    (ts as i64, random_tail(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encode_decode() {
        let u = mint(1_700_000_000_000);
        let s = to_crockford(&u);
        assert_eq!(s.len(), 26);
        let back = parse_crockford(&s).unwrap();
        assert_eq!(back, u);
    }

    #[test]
    fn pre_1970_clamp() {
        let u = mint(-1);
        assert_eq!(&u[..6], &[0, 0, 0, 0, 0, 0]);
    }
}
