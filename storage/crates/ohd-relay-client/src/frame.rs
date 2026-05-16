//! Client-side `TunnelFrame` codec for the relay QUIC tunnel.
//!
//! This is the storage / consumer side of the binary tunnel framing
//! documented in `relay/spec/relay-protocol.md` "Frame format" and at the
//! top of `relay/src/quic_tunnel.rs`. It was extracted from
//! `ohd-storage-server`'s inline `relay_client` module so the Android
//! uniffi binding and CORD can speak the same wire without depending on
//! the server binary crate.
//!
//! # Wire shape
//!
//! Per `relay/spec/relay-protocol.md` "Frame format" — big-endian, fixed
//! 12-byte header:
//!
//! ```text
//! 0       1       2       3       4
//! +-------+-------+-------+-------+
//! | MAGIC | TYPE  | FLAGS | RSVD  |   MAGIC = 0x4F ('O')
//! +-------+-------+-------+-------+
//! | SESSION_ID (4 bytes, BE u32)  |   0 = control / unbound
//! +-------+-------+-------+-------+
//! | PAYLOAD_LEN (4 bytes, BE u32) |   value capped at 65535
//! +-------+-------+-------+-------+
//! | PAYLOAD (PAYLOAD_LEN bytes)   |
//! +-------+-------+-------+-------+
//! ```
//!
//! # Wire-ABI contract
//!
//! This codec is the *consumer / storage* half of the relay's wire ABI.
//! The relay does **not** forward frames blindly — it decodes and
//! re-encodes every OPEN / OPEN_ACK / DATA / CLOSE envelope with its own
//! `ohd_relay::frame::TunnelFrame` codec (`relay/src/quic_tunnel.rs`
//! `session_reader_loop`, `relay/src/server.rs` `run_consumer_attach`).
//! So this module is **byte-for-byte identical** to that codec: same
//! magic, same 12-byte header, same u32 `payload_len`, same frame-type
//! discriminants. If the relay changes the wire format, this module needs
//! the matching edit — they are one protocol with two implementations.
//!
//! We embed the codec here (rather than depending on `ohd-relay::frame`)
//! because this crate intentionally has no path-dep on the relay crate.

use bytes::Bytes;

/// Magic byte at the head of every frame: ASCII `'O'` for OHD. One byte —
/// matching the relay's `ohd_relay::frame::MAGIC`.
pub const FRAME_MAGIC: u8 = 0x4F;

/// Fixed header size in bytes (MAGIC + TYPE + FLAGS + RSVD + SESSION_ID +
/// PAYLOAD_LEN = 1 + 1 + 1 + 1 + 4 + 4).
pub const FRAME_HEADER_LEN: usize = 1 + 1 + 1 + 1 + 4 + 4;

/// Maximum permitted payload length per frame.
///
/// Per the wire spec the on-wire `payload_len` is a 32-bit field, but the
/// value is capped at `u16::MAX` — payloads larger split into multiple
/// DATA frames. We enforce the cap on both encode and decode, matching
/// `ohd_relay::frame::MAX_PAYLOAD_LEN`.
pub const MAX_PAYLOAD_LEN: usize = u16::MAX as usize;

// ---------------------------------------------------------------------------
// FrameType
// ---------------------------------------------------------------------------

/// Tunnel frame type byte.
///
/// The discriminants match `relay/spec/relay-protocol.md` "Frame types" and
/// `ohd_relay::frame::FrameType` exactly — the relay parses these bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameType {
    Hello = 0x01,
    Open = 0x02,
    OpenAck = 0x03,
    OpenNack = 0x04,
    Data = 0x05,
    Close = 0x06,
    Ping = 0x07,
    Pong = 0x08,
    WindowUpdate = 0x0A,
}

impl FrameType {
    /// Parse a type byte. Unknown bytes yield [`FrameError::Other`].
    ///
    /// `0x09` (`WAKE_REQUEST`) is intentionally rejected: per the spec it
    /// is a push notification, not a tunnel frame.
    pub fn from_u8(b: u8) -> Result<Self, FrameError> {
        Ok(match b {
            0x01 => FrameType::Hello,
            0x02 => FrameType::Open,
            0x03 => FrameType::OpenAck,
            0x04 => FrameType::OpenNack,
            0x05 => FrameType::Data,
            0x06 => FrameType::Close,
            0x07 => FrameType::Ping,
            0x08 => FrameType::Pong,
            0x0A => FrameType::WindowUpdate,
            other => {
                return Err(FrameError::Other(format!(
                    "unknown frame type 0x{other:02x}"
                )))
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------

/// A decoded tunnel frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub frame_type: FrameType,
    pub session_id: u32,
    pub payload: Bytes,
}

/// Frame codec error.
#[derive(Debug)]
pub enum FrameError {
    /// The buffer does not yet hold a complete frame — read more bytes.
    Truncated,
    /// A structural decode failure (bad magic, unknown type, etc.).
    Other(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Truncated => write!(f, "truncated frame"),
            FrameError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for FrameError {}

/// Serialize a frame into a freshly-allocated buffer.
pub fn encode_frame(frame_type: FrameType, session_id: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(payload.len() <= MAX_PAYLOAD_LEN);
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.push(FRAME_MAGIC);
    buf.push(frame_type as u8);
    buf.push(0); // flags
    buf.push(0); // reserved
    buf.extend_from_slice(&session_id.to_be_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Parse one frame from the front of `buf`.
///
/// Returns the decoded [`Frame`] and the number of bytes consumed (header +
/// payload). [`FrameError::Truncated`] means the buffer is incomplete — the
/// caller should read more bytes and retry without discarding `buf`.
pub fn decode_one_frame(buf: &[u8]) -> Result<(Frame, usize), FrameError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FrameError::Truncated);
    }
    if buf[0] != FRAME_MAGIC {
        return Err(FrameError::Other(format!("bad magic: 0x{:02x}", buf[0])));
    }
    let frame_type = FrameType::from_u8(buf[1])?;
    if buf[2] != 0 {
        return Err(FrameError::Other(format!("non-zero flags: 0x{:02x}", buf[2])));
    }
    if buf[3] != 0 {
        return Err(FrameError::Other(format!(
            "non-zero reserved: 0x{:02x}",
            buf[3]
        )));
    }
    let session_id = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let payload_len = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(FrameError::Other(format!(
            "payload too large: {payload_len} bytes (max {MAX_PAYLOAD_LEN})"
        )));
    }
    let total = FRAME_HEADER_LEN + payload_len;
    if buf.len() < total {
        return Err(FrameError::Truncated);
    }
    let payload = Bytes::copy_from_slice(&buf[FRAME_HEADER_LEN..total]);
    Ok((
        Frame {
            frame_type,
            session_id,
            payload,
        },
        total,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_data() {
        let payload = b"hello, world";
        let bytes = encode_frame(FrameType::Data, 42, payload);
        let (frame, consumed) = decode_one_frame(&bytes).expect("decode");
        assert_eq!(consumed, bytes.len());
        assert_eq!(frame.frame_type, FrameType::Data);
        assert_eq!(frame.session_id, 42);
        assert_eq!(&frame.payload[..], payload);
    }

    #[test]
    fn frame_header_len_matches_relay() {
        // The relay's `ohd_relay::frame::HEADER_LEN` is 12. The two codecs
        // are one protocol; if this drifts the tunnel breaks.
        assert_eq!(FRAME_HEADER_LEN, 12);
    }

    #[test]
    fn frame_roundtrip_empty_payload() {
        for ft in [
            FrameType::Hello,
            FrameType::Ping,
            FrameType::Pong,
            FrameType::Open,
            FrameType::OpenAck,
            FrameType::OpenNack,
            FrameType::Close,
            FrameType::WindowUpdate,
        ] {
            let bytes = encode_frame(ft, 7, &[]);
            let (frame, consumed) = decode_one_frame(&bytes).expect("decode");
            assert_eq!(consumed, bytes.len());
            assert_eq!(consumed, FRAME_HEADER_LEN);
            assert_eq!(frame.frame_type, ft);
            assert_eq!(frame.session_id, 7);
            assert!(frame.payload.is_empty());
        }
    }

    #[test]
    fn frame_roundtrip_max_payload() {
        let payload = vec![0xCDu8; MAX_PAYLOAD_LEN];
        let bytes = encode_frame(FrameType::Data, 99, &payload);
        let (frame, consumed) = decode_one_frame(&bytes).expect("decode");
        assert_eq!(consumed, bytes.len());
        assert_eq!(frame.payload.len(), MAX_PAYLOAD_LEN);
    }

    #[test]
    fn frame_decode_truncated_header() {
        let bytes = encode_frame(FrameType::Open, 1, b"x");
        match decode_one_frame(&bytes[..FRAME_HEADER_LEN - 1]) {
            Err(FrameError::Truncated) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn frame_decode_truncated_payload() {
        let bytes = encode_frame(FrameType::Open, 1, b"x");
        // Header ok, payload missing one byte.
        match decode_one_frame(&bytes[..bytes.len() - 1]) {
            Err(FrameError::Truncated) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn frame_decode_two_in_one_buffer() {
        let mut all = encode_frame(FrameType::OpenAck, 7, &[]);
        all.extend_from_slice(&encode_frame(FrameType::Data, 7, b"abc"));
        let (f1, c1) = decode_one_frame(&all).expect("decode 1");
        assert_eq!(f1.frame_type, FrameType::OpenAck);
        let (f2, c2) = decode_one_frame(&all[c1..]).expect("decode 2");
        assert_eq!(f2.frame_type, FrameType::Data);
        assert_eq!(&f2.payload[..], b"abc");
        assert_eq!(c1 + c2, all.len());
    }

    #[test]
    fn frame_decode_bad_magic() {
        let mut bytes = encode_frame(FrameType::Data, 1, b"x");
        bytes[0] = b'X';
        match decode_one_frame(&bytes) {
            Err(FrameError::Other(_)) => {}
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn frame_decode_unknown_type() {
        let mut bytes = encode_frame(FrameType::Data, 1, b"x");
        bytes[1] = 0xEE;
        match decode_one_frame(&bytes) {
            Err(FrameError::Other(_)) => {}
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn frame_decode_nonzero_reserved() {
        let mut bytes = encode_frame(FrameType::Data, 1, b"x");
        bytes[3] = 0xAA;
        match decode_one_frame(&bytes) {
            Err(FrameError::Other(_)) => {}
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
