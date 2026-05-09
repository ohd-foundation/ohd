//! Sample-block codecs.
//!
//! Implements the on-disk encodings declared in `spec/storage-format.md`
//! "Sample blocks". Each block encodes `(t_offset_ms, value)` pairs where
//! `t_offset_ms` is relative to the block's `t0_ms`.
//!
//! # Encodings
//!
//! ## Encoding `1` — delta-zigzag-varint timestamps + float32 values, zstd
//!
//! Layout (uncompressed):
//!
//! ```text
//! varint(sample_count)
//!   || zigzag_varint(dt_0)        || float32(v_0)
//!   || zigzag_varint(dt_1 - dt_0) || float32(v_1)
//!   || ...
//! ```
//!
//! Compressed with zstd level 3.
//!
//! ## Encoding `2` — delta-zigzag-varint timestamps + int16 quantized + scale
//!
//! Layout (uncompressed):
//!
//! ```text
//! varint(sample_count)
//!   || float32(scale) || float32(offset)
//!   || zigzag_varint(dt_0)        || int16(q_0)
//!   || zigzag_varint(dt_1 - dt_0) || int16(q_1)
//!   || ...
//! ```
//!
//! Decoded value: `q_i * scale + offset`.
//!
//! Useful for integer-quantized streams (HR bpm, step counts) at ~half the
//! size of encoding 1.
//!
//! # Determinism
//!
//! The encoders are byte-deterministic: same input → byte-identical output.
//! The varint and zigzag routines are order-of-operations-fixed; zstd is
//! invoked at level 3 with default parameters. The conformance corpus
//! (`spec/conformance.md` "On-disk format conformance") asserts this property.

use crate::{Error, Result};

/// Encoding ID for delta-zigzag-varint timestamps + float32 values.
pub const ENCODING_F32: i32 = 1;
/// Encoding ID for delta-zigzag-varint timestamps + int16 quantized values.
pub const ENCODING_I16: i32 = 2;

/// One decoded `(t_offset_ms, value)` sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Offset (ms) from the block's `t0_ms`.
    pub t_offset_ms: i64,
    /// Decoded value.
    pub value: f64,
}

/// Encode a sample block under encoding 1 (`float32` values).
///
/// `samples` may be empty (yields just the `varint(0)` zstd-compressed). The
/// caller supplies offsets relative to the block's `t0_ms`; the codec does not
/// know about absolute timestamps.
pub fn encode_f32(samples: &[Sample]) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(samples.len() * 8 + 4);
    write_varint(&mut buf, samples.len() as u64);
    let mut prev_dt: i64 = 0;
    for s in samples {
        let delta = s.t_offset_ms - prev_dt;
        write_zigzag_varint(&mut buf, delta);
        buf.extend_from_slice(&(s.value as f32).to_le_bytes());
        prev_dt = s.t_offset_ms;
    }
    zstd_compress(&buf)
}

/// Decode a sample block produced by [`encode_f32`]. Produces sample offsets
/// relative to the block's `t0_ms`.
pub fn decode_f32(data: &[u8]) -> Result<Vec<Sample>> {
    let raw = zstd_decompress(data)?;
    let mut cursor = Cursor::new(&raw);
    let n = cursor.read_varint()? as usize;
    let mut out = Vec::with_capacity(n);
    let mut prev_dt: i64 = 0;
    for _ in 0..n {
        let delta = cursor.read_zigzag_varint()?;
        let v = cursor.read_f32_le()? as f64;
        let t = prev_dt + delta;
        out.push(Sample {
            t_offset_ms: t,
            value: v,
        });
        prev_dt = t;
    }
    Ok(out)
}

/// Encode a sample block under encoding 2 (int16 quantized).
///
/// Each sample's `value` is encoded as `int16((value - offset) / scale)`. The
/// caller chooses `scale` and `offset`; pick `scale=1.0, offset=0.0` for
/// integer streams. The codec rounds to nearest with ties-to-even and clamps
/// to `i16::MIN..=i16::MAX` (out-of-range samples saturate; callers wanting
/// strict checks should validate before calling).
pub fn encode_i16(samples: &[Sample], scale: f32, offset: f32) -> Result<Vec<u8>> {
    if !scale.is_finite() || scale == 0.0 {
        return Err(Error::InvalidArgument(format!(
            "encode_i16: scale must be finite and non-zero, got {scale}"
        )));
    }
    if !offset.is_finite() {
        return Err(Error::InvalidArgument(format!(
            "encode_i16: offset must be finite, got {offset}"
        )));
    }
    let mut buf = Vec::with_capacity(samples.len() * 4 + 12);
    write_varint(&mut buf, samples.len() as u64);
    buf.extend_from_slice(&scale.to_le_bytes());
    buf.extend_from_slice(&offset.to_le_bytes());
    let mut prev_dt: i64 = 0;
    for s in samples {
        let delta = s.t_offset_ms - prev_dt;
        write_zigzag_varint(&mut buf, delta);
        let q = quantize_to_i16(s.value as f32, scale, offset);
        buf.extend_from_slice(&q.to_le_bytes());
        prev_dt = s.t_offset_ms;
    }
    zstd_compress(&buf)
}

/// Decode a sample block produced by [`encode_i16`].
pub fn decode_i16(data: &[u8]) -> Result<Vec<Sample>> {
    let raw = zstd_decompress(data)?;
    let mut cursor = Cursor::new(&raw);
    let n = cursor.read_varint()? as usize;
    let scale = cursor.read_f32_le()?;
    let offset = cursor.read_f32_le()?;
    let mut out = Vec::with_capacity(n);
    let mut prev_dt: i64 = 0;
    for _ in 0..n {
        let delta = cursor.read_zigzag_varint()?;
        let q = cursor.read_i16_le()?;
        let v = (q as f32) * scale + offset;
        let t = prev_dt + delta;
        out.push(Sample {
            t_offset_ms: t,
            value: v as f64,
        });
        prev_dt = t;
    }
    Ok(out)
}

/// Decode any supported encoding. Returns the decoded samples; callers can
/// feed those into [`crate::events::SampleBlockRef`] tooling.
pub fn decode(encoding: i32, data: &[u8]) -> Result<Vec<Sample>> {
    match encoding {
        ENCODING_F32 => decode_f32(data),
        ENCODING_I16 => decode_i16(data),
        other => Err(Error::UnsupportedEncoding(other)),
    }
}

// ============================================================================
// Internals — varint, zigzag, zstd, cursor
// ============================================================================

fn write_varint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

fn write_zigzag_varint(buf: &mut Vec<u8>, v: i64) {
    // Zigzag: (n << 1) ^ (n >> 63) — wrapping_shl avoids UB on i64::MIN.
    let zz = ((v as i64).wrapping_shl(1) ^ ((v as i64) >> 63)) as u64;
    write_varint(buf, zz);
}

fn quantize_to_i16(v: f32, scale: f32, offset: f32) -> i16 {
    let q = ((v - offset) / scale).round();
    if q.is_nan() {
        return 0;
    }
    if q >= i16::MAX as f32 {
        i16::MAX
    } else if q <= i16::MIN as f32 {
        i16::MIN
    } else {
        q as i16
    }
}

fn zstd_compress(data: &[u8]) -> Result<Vec<u8>> {
    // Level 3 keeps the codec spec-pinned (see module docs). zstd::bulk's
    // single-shot compressor is deterministic given identical level + dict =
    // none + identical input; the conformance harness cross-checks this.
    zstd::bulk::compress(data, 3)
        .map_err(|e| Error::Internal(anyhow::anyhow!("zstd compress: {e}")))
}

fn zstd_decompress(data: &[u8]) -> Result<Vec<u8>> {
    // 256 MiB ceiling matches the spec's payload limits; an attacker-supplied
    // sample block can't blow up our address space.
    const MAX_DECODED: usize = 256 * 1024 * 1024;
    zstd::bulk::decompress(data, MAX_DECODED)
        .map_err(|e| Error::Internal(anyhow::anyhow!("zstd decompress: {e}")))
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_byte(&mut self) -> Result<u8> {
        if self.pos >= self.buf.len() {
            return Err(Error::InvalidArgument(
                "sample-block: unexpected end of buffer".into(),
            ));
        }
        let b = self.buf[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return Err(Error::InvalidArgument(
                "sample-block: unexpected end of buffer".into(),
            ));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_varint(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let b = self.read_byte()?;
            // Bound the shift to 64 bits; anything > 9 bytes for a u64 varint
            // is malformed.
            if shift >= 64 {
                return Err(Error::InvalidArgument(
                    "sample-block: varint overflow".into(),
                ));
            }
            result |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    fn read_zigzag_varint(&mut self) -> Result<i64> {
        let zz = self.read_varint()?;
        Ok(((zz >> 1) as i64) ^ -((zz & 1) as i64))
    }

    fn read_f32_le(&mut self) -> Result<f32> {
        let b = self.read_bytes(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i16_le(&mut self) -> Result<i16> {
        let b = self.read_bytes(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn close_enough(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn varint_roundtrip() {
        for v in [0u64, 1, 127, 128, 0xFFFF, 0xFFFF_FFFF, u64::MAX] {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            let decoded = Cursor::new(&buf).read_varint().expect("decode");
            assert_eq!(decoded, v);
        }
    }

    #[test]
    fn zigzag_roundtrip() {
        for v in [0i64, 1, -1, 127, -128, i64::MIN, i64::MAX, 1234567890] {
            let mut buf = Vec::new();
            write_zigzag_varint(&mut buf, v);
            let decoded = Cursor::new(&buf).read_zigzag_varint().expect("decode");
            assert_eq!(decoded, v);
        }
    }

    #[test]
    fn f32_empty_roundtrip() {
        let bytes = encode_f32(&[]).unwrap();
        let decoded = decode_f32(&bytes).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn f32_small_roundtrip() {
        let samples = vec![
            Sample {
                t_offset_ms: 0,
                value: 72.5,
            },
            Sample {
                t_offset_ms: 1000,
                value: 73.0,
            },
            Sample {
                t_offset_ms: 2000,
                value: 75.25,
            },
            Sample {
                t_offset_ms: 4000,
                value: 71.75,
            },
        ];
        let bytes = encode_f32(&samples).unwrap();
        let decoded = decode_f32(&bytes).unwrap();
        assert_eq!(decoded.len(), samples.len());
        for (a, b) in decoded.iter().zip(samples.iter()) {
            assert_eq!(a.t_offset_ms, b.t_offset_ms);
            assert!(close_enough(a.value, b.value, 1e-3));
        }
    }

    #[test]
    fn f32_dense_roundtrip() {
        // 900 samples (15 min @ 1Hz) — one of the spec's reference sizes.
        let samples: Vec<Sample> = (0..900)
            .map(|i| Sample {
                t_offset_ms: i as i64 * 1000,
                value: 60.0 + ((i as f64) * 0.05).sin() * 10.0,
            })
            .collect();
        let bytes = encode_f32(&samples).unwrap();
        // Compressed should fit comfortably in <8KB; without delta+zstd a raw
        // f32+i64 stream would be 900*12 = 10.8 KB.
        assert!(bytes.len() < 8 * 1024, "encoded {} bytes", bytes.len());
        let decoded = decode_f32(&bytes).unwrap();
        assert_eq!(decoded.len(), 900);
        for (a, b) in decoded.iter().zip(samples.iter()) {
            assert_eq!(a.t_offset_ms, b.t_offset_ms);
            assert!(close_enough(a.value, b.value, 1e-3));
        }
    }

    #[test]
    fn f32_deterministic() {
        // Same input → byte-identical output. Conformance-load-bearing.
        let samples = vec![
            Sample {
                t_offset_ms: 0,
                value: 1.0,
            },
            Sample {
                t_offset_ms: 100,
                value: 2.0,
            },
            Sample {
                t_offset_ms: 200,
                value: 3.0,
            },
        ];
        let a = encode_f32(&samples).unwrap();
        let b = encode_f32(&samples).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn i16_simple_roundtrip() {
        let samples = vec![
            Sample {
                t_offset_ms: 0,
                value: 60.0,
            },
            Sample {
                t_offset_ms: 1000,
                value: 65.0,
            },
            Sample {
                t_offset_ms: 2000,
                value: 70.0,
            },
        ];
        let bytes = encode_i16(&samples, 1.0, 0.0).unwrap();
        let decoded = decode_i16(&bytes).unwrap();
        assert_eq!(decoded.len(), samples.len());
        for (a, b) in decoded.iter().zip(samples.iter()) {
            assert_eq!(a.t_offset_ms, b.t_offset_ms);
            assert!(close_enough(a.value, b.value, 0.6));
        }
    }

    #[test]
    fn i16_with_scale_offset() {
        // Stream of glucose mg/dL values quantized at 0.1 mg/dL resolution
        // around an offset of 100. Scale 0.1 means q in [-32768, 32767]
        // covers ±3276.7 mg/dL — ample headroom.
        let samples: Vec<Sample> = (0..100)
            .map(|i| Sample {
                t_offset_ms: i as i64 * 1000,
                value: 100.0 + (i as f64) * 0.5,
            })
            .collect();
        let bytes = encode_i16(&samples, 0.1, 100.0).unwrap();
        let decoded = decode_i16(&bytes).unwrap();
        for (a, b) in decoded.iter().zip(samples.iter()) {
            assert_eq!(a.t_offset_ms, b.t_offset_ms);
            assert!(close_enough(a.value, b.value, 0.05));
        }
    }

    #[test]
    fn i16_deterministic() {
        let samples = vec![
            Sample {
                t_offset_ms: 0,
                value: 60.0,
            },
            Sample {
                t_offset_ms: 1000,
                value: 65.0,
            },
        ];
        let a = encode_i16(&samples, 1.0, 0.0).unwrap();
        let b = encode_i16(&samples, 1.0, 0.0).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn i16_zero_scale_rejected() {
        let r = encode_i16(
            &[Sample {
                t_offset_ms: 0,
                value: 1.0,
            }],
            0.0,
            0.0,
        );
        assert!(r.is_err());
    }

    #[test]
    fn decode_dispatches() {
        let f32_data = encode_f32(&[Sample {
            t_offset_ms: 0,
            value: 42.0,
        }])
        .unwrap();
        let i16_data = encode_i16(
            &[Sample {
                t_offset_ms: 0,
                value: 42.0,
            }],
            1.0,
            0.0,
        )
        .unwrap();
        let out_f32 = decode(ENCODING_F32, &f32_data).unwrap();
        let out_i16 = decode(ENCODING_I16, &i16_data).unwrap();
        assert_eq!(out_f32.len(), 1);
        assert_eq!(out_i16.len(), 1);
        assert!(matches!(
            decode(99, &f32_data),
            Err(Error::UnsupportedEncoding(99))
        ));
    }
}
