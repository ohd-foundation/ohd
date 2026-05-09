//! Binary `TunnelFrame` codec for the relay tunnel protocol.
//!
//! Wire format (big-endian, per `spec/relay-protocol.md` "Frame format"):
//!
//! ```text
//! 0       1       2       3       4
//! +-------+-------+-------+-------+
//! | MAGIC | TYPE  | FLAGS | RSVD  |   MAGIC = 0x4F ('O')
//! +-------+-------+-------+-------+
//! | SESSION_ID (4 bytes, BE u32)  |   0 = control / unbound
//! +-------+-------+-------+-------+
//! | PAYLOAD_LEN (4 bytes, BE u32) |   max 65535
//! +-------+-------+-------+-------+
//! | PAYLOAD (PAYLOAD_LEN bytes)   |
//! +-------+-------+-------+-------+
//! ```
//!
//! Header is fixed at 12 bytes; payload is length-prefixed. The codec is
//! deliberately simple — no streaming reassembly above this layer; the caller
//! is expected to feed whole frames in (over a WebSocket message boundary,
//! or after a length-prefixed read on a raw stream).

use bytes::{Buf, BufMut, Bytes, BytesMut};

/// Magic byte at the head of every frame: ASCII `'O'` for OHD.
pub const MAGIC: u8 = 0x4F;

/// Fixed header size in bytes (MAGIC + TYPE + FLAGS + RSVD + SESSION_ID + PAYLOAD_LEN).
pub const HEADER_LEN: usize = 12;

/// Maximum permitted payload length per frame.
///
/// Per the wire spec: "Big-endian uint32; payload byte length. Max 65535 —
/// payloads larger split into multiple DATA frames." The 32-bit width on the
/// wire leaves room for future expansion, but the spec caps the value at
/// `u16::MAX` and we enforce it on both encode and decode.
pub const MAX_PAYLOAD_LEN: usize = u16::MAX as usize;

// ---------------------------------------------------------------------------
// FrameType
// ---------------------------------------------------------------------------

/// Frame type byte.
///
/// `WAKE_REQUEST` (0x09) is intentionally absent: per the spec it is a push
/// notification payload, not a tunnel frame. Including it here would invite
/// senders to emit it on the tunnel.
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
    pub fn from_u8(b: u8) -> Result<Self, FrameError> {
        match b {
            0x01 => Ok(Self::Hello),
            0x02 => Ok(Self::Open),
            0x03 => Ok(Self::OpenAck),
            0x04 => Ok(Self::OpenNack),
            0x05 => Ok(Self::Data),
            0x06 => Ok(Self::Close),
            0x07 => Ok(Self::Ping),
            0x08 => Ok(Self::Pong),
            // 0x09 (WAKE_REQUEST) is push-only; reject on the tunnel.
            0x0A => Ok(Self::WindowUpdate),
            other => Err(FrameError::UnknownType(other)),
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Whether this frame type carries a session ID. Control frames whose
    /// `SESSION_ID == 0` is canonical (HELLO, PING, PONG) return `false`.
    pub fn is_session_bound(self) -> bool {
        matches!(
            self,
            FrameType::Open
                | FrameType::OpenAck
                | FrameType::OpenNack
                | FrameType::Data
                | FrameType::Close
                | FrameType::WindowUpdate
        )
    }
}

// ---------------------------------------------------------------------------
// TunnelFrame
// ---------------------------------------------------------------------------

/// A parsed (or to-be-encoded) tunnel frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelFrame {
    pub frame_type: FrameType,
    pub flags: u8,
    pub session_id: u32,
    pub payload: Bytes,
}

impl TunnelFrame {
    pub fn new(frame_type: FrameType, session_id: u32, payload: impl Into<Bytes>) -> Self {
        Self {
            frame_type,
            flags: 0,
            session_id,
            payload: payload.into(),
        }
    }

    pub fn with_flags(mut self, flags: u8) -> Self {
        self.flags = flags;
        self
    }

    /// Total wire size (header + payload).
    pub fn wire_size(&self) -> usize {
        HEADER_LEN + self.payload.len()
    }

    // -- Convenience constructors for the common control frames --

    pub fn hello(payload: impl Into<Bytes>) -> Self {
        Self::new(FrameType::Hello, 0, payload)
    }

    pub fn ping(payload: impl Into<Bytes>) -> Self {
        Self::new(FrameType::Ping, 0, payload)
    }

    pub fn pong(payload: impl Into<Bytes>) -> Self {
        Self::new(FrameType::Pong, 0, payload)
    }

    pub fn open(session_id: u32, grant_token_preview: impl Into<Bytes>) -> Self {
        Self::new(FrameType::Open, session_id, grant_token_preview)
    }

    pub fn open_ack(session_id: u32) -> Self {
        Self::new(FrameType::OpenAck, session_id, Bytes::new())
    }

    pub fn open_nack(session_id: u32, reason: impl Into<Bytes>) -> Self {
        Self::new(FrameType::OpenNack, session_id, reason)
    }

    pub fn data(session_id: u32, payload: impl Into<Bytes>) -> Self {
        Self::new(FrameType::Data, session_id, payload)
    }

    pub fn close(session_id: u32, reason: impl Into<Bytes>) -> Self {
        Self::new(FrameType::Close, session_id, reason)
    }

    pub fn window_update(session_id: u32, increment: u32) -> Self {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32(increment);
        Self::new(FrameType::WindowUpdate, session_id, buf.freeze())
    }

    // -- Encode --

    /// Serialize into a freshly-allocated `Bytes`.
    pub fn encode(&self) -> Result<Bytes, FrameError> {
        let mut buf = BytesMut::with_capacity(self.wire_size());
        self.encode_into(&mut buf)?;
        Ok(buf.freeze())
    }

    /// Serialize, appending to the provided buffer.
    pub fn encode_into(&self, buf: &mut BytesMut) -> Result<(), FrameError> {
        if self.payload.len() > MAX_PAYLOAD_LEN {
            return Err(FrameError::PayloadTooLarge {
                len: self.payload.len(),
                max: MAX_PAYLOAD_LEN,
            });
        }
        buf.reserve(self.wire_size());
        buf.put_u8(MAGIC);
        buf.put_u8(self.frame_type.as_u8());
        buf.put_u8(self.flags);
        buf.put_u8(0); // RSVD
        buf.put_u32(self.session_id);
        buf.put_u32(self.payload.len() as u32);
        buf.extend_from_slice(&self.payload);
        Ok(())
    }

    // -- Decode --

    /// Parse exactly one frame from `bytes`. The slice must contain the full
    /// header and payload; surplus bytes return `FrameError::TrailingBytes`.
    /// To parse one frame and continue reading, use [`TunnelFrame::decode_one`].
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        let (frame, consumed) = Self::decode_one(bytes)?;
        if consumed != bytes.len() {
            return Err(FrameError::TrailingBytes {
                consumed,
                total: bytes.len(),
            });
        }
        Ok(frame)
    }

    /// Parse one frame, returning the frame and the number of bytes consumed.
    pub fn decode_one(bytes: &[u8]) -> Result<(Self, usize), FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::Truncated {
                have: bytes.len(),
                need: HEADER_LEN,
            });
        }
        let mut hdr = &bytes[..HEADER_LEN];
        let magic = hdr.get_u8();
        if magic != MAGIC {
            return Err(FrameError::BadMagic(magic));
        }
        let type_byte = hdr.get_u8();
        let frame_type = FrameType::from_u8(type_byte)?;
        let flags = hdr.get_u8();
        let rsvd = hdr.get_u8();
        if rsvd != 0 {
            return Err(FrameError::ReservedNonZero(rsvd));
        }
        let session_id = hdr.get_u32();
        let payload_len = hdr.get_u32() as usize;

        if payload_len > MAX_PAYLOAD_LEN {
            return Err(FrameError::PayloadTooLarge {
                len: payload_len,
                max: MAX_PAYLOAD_LEN,
            });
        }

        let total = HEADER_LEN + payload_len;
        if bytes.len() < total {
            return Err(FrameError::Truncated {
                have: bytes.len(),
                need: total,
            });
        }

        // Validate session_id semantics for control frames. We are lenient:
        // we accept session-bound frames with id 0 (some peers may use that as
        // a sentinel) but log/refuse session ids on canonical control frames
        // (HELLO/PING/PONG must use 0). This is a sanity check, not gospel.
        if !frame_type.is_session_bound() && session_id != 0 {
            return Err(FrameError::ControlFrameWithSession {
                frame_type,
                session_id,
            });
        }

        let payload = Bytes::copy_from_slice(&bytes[HEADER_LEN..total]);
        Ok((
            Self {
                frame_type,
                flags,
                session_id,
                payload,
            },
            total,
        ))
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame truncated: have {have} bytes, need {need}")]
    Truncated { have: usize, need: usize },

    #[error("bad magic byte: 0x{0:02x} (expected 0x4F)")]
    BadMagic(u8),

    #[error("unknown frame type: 0x{0:02x}")]
    UnknownType(u8),

    #[error("reserved byte must be zero, got 0x{0:02x}")]
    ReservedNonZero(u8),

    #[error("payload too large: {len} bytes (max {max})")]
    PayloadTooLarge { len: usize, max: usize },

    #[error("trailing bytes after frame: consumed {consumed} of {total}")]
    TrailingBytes { consumed: usize, total: usize },

    #[error("control frame {frame_type:?} carried non-zero session id {session_id}")]
    ControlFrameWithSession {
        frame_type: FrameType,
        session_id: u32,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(frame: TunnelFrame) {
        let encoded = frame.encode().expect("encode");
        let decoded = TunnelFrame::decode(&encoded).expect("decode");
        assert_eq!(frame, decoded);
        assert_eq!(encoded.len(), frame.wire_size());
    }

    #[test]
    fn roundtrip_hello() {
        roundtrip(TunnelFrame::hello(Bytes::from_static(b"caps=v1")));
    }

    #[test]
    fn roundtrip_ping_zero_payload() {
        roundtrip(TunnelFrame::ping(Bytes::new()));
    }

    #[test]
    fn roundtrip_data_with_session() {
        let payload = vec![0xABu8; 4096];
        roundtrip(TunnelFrame::data(42, payload));
    }

    #[test]
    fn roundtrip_open_with_token_preview() {
        roundtrip(TunnelFrame::open(7, Bytes::from_static(b"ohdg_preview")));
    }

    #[test]
    fn roundtrip_open_ack() {
        roundtrip(TunnelFrame::open_ack(7));
    }

    #[test]
    fn roundtrip_open_nack_with_reason() {
        roundtrip(TunnelFrame::open_nack(
            7,
            Bytes::from_static(b"INVALID_TOKEN"),
        ));
    }

    #[test]
    fn roundtrip_close_with_reason() {
        roundtrip(TunnelFrame::close(7, Bytes::from_static(b"DONE")));
    }

    #[test]
    fn roundtrip_window_update() {
        roundtrip(TunnelFrame::window_update(11, 65_536));
    }

    #[test]
    fn roundtrip_max_payload() {
        let payload = vec![0xCDu8; MAX_PAYLOAD_LEN];
        roundtrip(TunnelFrame::data(99, payload));
    }

    #[test]
    fn encode_rejects_oversize_payload() {
        let too_big = vec![0u8; MAX_PAYLOAD_LEN + 1];
        let frame = TunnelFrame::data(1, too_big);
        let err = frame.encode().unwrap_err();
        assert!(matches!(err, FrameError::PayloadTooLarge { .. }));
    }

    #[test]
    fn decode_rejects_truncated_header() {
        let bytes = [0x4F, 0x01, 0x00]; // 3 bytes, < HEADER_LEN
        let err = TunnelFrame::decode(&bytes).unwrap_err();
        assert!(matches!(err, FrameError::Truncated { .. }));
    }

    #[test]
    fn decode_rejects_truncated_payload() {
        // Header claims 100-byte payload, but body is empty.
        let mut buf = BytesMut::new();
        buf.put_u8(MAGIC);
        buf.put_u8(FrameType::Data.as_u8());
        buf.put_u8(0);
        buf.put_u8(0);
        buf.put_u32(1);
        buf.put_u32(100);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::Truncated { .. }));
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut buf = BytesMut::new();
        buf.put_u8(0x42);
        buf.put_u8(FrameType::Data.as_u8());
        buf.put_u8(0);
        buf.put_u8(0);
        buf.put_u32(0);
        buf.put_u32(0);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::BadMagic(0x42)));
    }

    #[test]
    fn decode_rejects_unknown_type() {
        let mut buf = BytesMut::new();
        buf.put_u8(MAGIC);
        buf.put_u8(0xEE);
        buf.put_u8(0);
        buf.put_u8(0);
        buf.put_u32(0);
        buf.put_u32(0);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::UnknownType(0xEE)));
    }

    #[test]
    fn decode_rejects_wake_request_on_tunnel() {
        // 0x09 is push-only.
        let mut buf = BytesMut::new();
        buf.put_u8(MAGIC);
        buf.put_u8(0x09);
        buf.put_u8(0);
        buf.put_u8(0);
        buf.put_u32(0);
        buf.put_u32(0);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::UnknownType(0x09)));
    }

    #[test]
    fn decode_rejects_nonzero_reserved() {
        let mut buf = BytesMut::new();
        buf.put_u8(MAGIC);
        buf.put_u8(FrameType::Ping.as_u8());
        buf.put_u8(0);
        buf.put_u8(0xAA);
        buf.put_u32(0);
        buf.put_u32(0);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::ReservedNonZero(0xAA)));
    }

    #[test]
    fn decode_rejects_oversize_payload_len() {
        let mut buf = BytesMut::new();
        buf.put_u8(MAGIC);
        buf.put_u8(FrameType::Data.as_u8());
        buf.put_u8(0);
        buf.put_u8(0);
        buf.put_u32(1);
        buf.put_u32((MAX_PAYLOAD_LEN as u32) + 1);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::PayloadTooLarge { .. }));
    }

    #[test]
    fn decode_rejects_control_frame_with_session() {
        let mut buf = BytesMut::new();
        buf.put_u8(MAGIC);
        buf.put_u8(FrameType::Ping.as_u8());
        buf.put_u8(0);
        buf.put_u8(0);
        buf.put_u32(123); // illegal: PING is unbound
        buf.put_u32(0);
        let err = TunnelFrame::decode(&buf).unwrap_err();
        assert!(matches!(err, FrameError::ControlFrameWithSession { .. }));
    }

    #[test]
    fn decode_one_returns_consumed_count() {
        let frame = TunnelFrame::data(3, Bytes::from_static(b"hello"));
        let mut combined = BytesMut::new();
        frame.encode_into(&mut combined).unwrap();
        // Append trailing junk that decode_one should leave alone.
        combined.extend_from_slice(b"junk");
        let (parsed, consumed) = TunnelFrame::decode_one(&combined).unwrap();
        assert_eq!(parsed, frame);
        assert_eq!(consumed, frame.wire_size());
    }

    #[test]
    fn decode_strict_rejects_trailing_bytes() {
        let frame = TunnelFrame::ping(Bytes::new());
        let mut combined = BytesMut::new();
        frame.encode_into(&mut combined).unwrap();
        combined.extend_from_slice(b"x");
        let err = TunnelFrame::decode(&combined).unwrap_err();
        assert!(matches!(err, FrameError::TrailingBytes { .. }));
    }

    #[test]
    fn frame_type_session_bound_classification() {
        assert!(!FrameType::Hello.is_session_bound());
        assert!(!FrameType::Ping.is_session_bound());
        assert!(!FrameType::Pong.is_session_bound());
        assert!(FrameType::Open.is_session_bound());
        assert!(FrameType::Data.is_session_bound());
        assert!(FrameType::WindowUpdate.is_session_bound());
    }
}
