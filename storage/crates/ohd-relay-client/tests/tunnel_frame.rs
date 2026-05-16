//! `TunnelFrame` codec round-trip integration tests.
//!
//! Exercises the public `encode_frame` / `decode_one_frame` surface the
//! Android binding and CORD consume, including the multi-frame-in-one-buffer
//! quirk that the buffered reader in `tunnel.rs` has to handle.

use ohd_relay_client::{decode_one_frame, encode_frame, FrameError, FrameType};

const ALL_TYPES: &[FrameType] = &[
    FrameType::Hello,
    FrameType::Ping,
    FrameType::Pong,
    FrameType::Open,
    FrameType::OpenAck,
    FrameType::OpenNack,
    FrameType::Data,
    FrameType::Close,
    FrameType::WindowUpdate,
];

#[test]
fn roundtrip_every_frame_type() {
    for &ft in ALL_TYPES {
        let payload = b"opaque-tls-record-bytes";
        let bytes = encode_frame(ft, 0xDEAD_BEEF, payload);
        let (frame, consumed) = decode_one_frame(&bytes).expect("decode");
        assert_eq!(consumed, bytes.len());
        assert_eq!(frame.frame_type, ft);
        assert_eq!(frame.session_id, 0xDEAD_BEEF);
        assert_eq!(&frame.payload[..], payload);
    }
}

#[test]
fn roundtrip_empty_and_max_payload() {
    let empty = encode_frame(FrameType::OpenAck, 1, &[]);
    let (f, c) = decode_one_frame(&empty).unwrap();
    assert!(f.payload.is_empty());
    assert_eq!(c, empty.len());

    let big = vec![0x5Au8; u16::MAX as usize];
    let encoded = encode_frame(FrameType::Data, 2, &big);
    let (f, c) = decode_one_frame(&encoded).unwrap();
    assert_eq!(f.payload.len(), u16::MAX as usize);
    assert_eq!(c, encoded.len());
}

#[test]
fn streamed_buffer_decodes_frame_by_frame() {
    // Simulate a quinn read chunk carrying three back-to-back frames — the
    // exact wire quirk `read_one_frame_buffered` exists to handle.
    let mut wire = Vec::new();
    wire.extend_from_slice(&encode_frame(FrameType::Open, 7, b"ohdg_tok"));
    wire.extend_from_slice(&encode_frame(FrameType::OpenAck, 7, &[]));
    wire.extend_from_slice(&encode_frame(FrameType::Data, 7, b"payload"));

    let mut offset = 0;
    let mut decoded = Vec::new();
    while offset < wire.len() {
        let (frame, consumed) = decode_one_frame(&wire[offset..]).expect("decode");
        decoded.push(frame.frame_type);
        offset += consumed;
    }
    assert_eq!(offset, wire.len());
    assert_eq!(
        decoded,
        vec![FrameType::Open, FrameType::OpenAck, FrameType::Data]
    );
}

#[test]
fn partial_buffer_reports_truncated() {
    let full = encode_frame(FrameType::Data, 3, b"abcdefgh");
    // Every prefix shorter than the full frame must report Truncated.
    for cut in 0..full.len() {
        match decode_one_frame(&full[..cut]) {
            Err(FrameError::Truncated) => {}
            other => panic!("prefix len {cut}: expected Truncated, got {other:?}"),
        }
    }
    // The full frame decodes.
    assert!(decode_one_frame(&full).is_ok());
}

#[test]
fn corrupt_magic_rejected() {
    let mut bytes = encode_frame(FrameType::Ping, 0, &[]);
    bytes[0] ^= 0xFF;
    match decode_one_frame(&bytes) {
        Err(FrameError::Other(_)) => {}
        other => panic!("expected Other, got {other:?}"),
    }
}
