//! Sample-block read / write helpers.
//!
//! Writes one row to `event_samples` per block; the codec lives in
//! [`crate::sample_codec`]. The OHDC server-streaming `ReadSamples` RPC
//! dispatches to [`read_samples_decoded`] which:
//!
//! 1. Resolves the event by ULID.
//! 2. Resolves the channel against the event's type registry.
//! 3. Fetches every block on `(event_id, channel_id)` ordered by `block_index`.
//! 4. Decodes each block via [`sample_codec::decode`] and yields the
//!    absolute-timestamped `(t_ms, value)` samples.
//!
//! The optional `[from_ms, to_ms]` slice is honoured by skipping blocks whose
//! `[t0_ms, t1_ms]` range lies entirely outside the requested window and, for
//! blocks that straddle the window boundary, filtering decoded samples.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::events::SampleBlockRef;
use crate::registry;
use crate::sample_codec;
use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// One decoded sample with an absolute timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AbsoluteSample {
    /// Absolute Unix milliseconds (signed; pre-1970 supported).
    pub t_ms: i64,
    /// Decoded value.
    pub value: f64,
}

/// Insert one `event_samples` row.
///
/// `block_index` is the position of the block within the channel; callers that
/// only ever write one block per `(event, channel)` may pass 0.
pub fn insert_sample_block(
    tx: &Transaction<'_>,
    event_id: i64,
    channel_id: i64,
    block_index: i32,
    t0_ms: i64,
    t1_ms: i64,
    sample_count: i32,
    encoding: i32,
    data: &[u8],
) -> Result<()> {
    tx.execute(
        "INSERT INTO event_samples
            (event_id, channel_id, block_index, t0_ms, t1_ms,
             sample_count, encoding, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event_id,
            channel_id,
            block_index,
            t0_ms,
            t1_ms,
            sample_count,
            encoding,
            data,
        ],
    )?;
    Ok(())
}

/// List all sample-block refs for an event. Used by [`crate::events`] to
/// populate `Event.sample_blocks` when fetching events.
pub fn list_blocks(conn: &Connection, event_id: i64) -> Result<Vec<SampleBlockRef>> {
    let mut stmt = conn.prepare(
        "SELECT c.path, s.t0_ms, s.t1_ms, s.sample_count, s.encoding
           FROM event_samples s
           JOIN channels c ON c.id = s.channel_id
          WHERE s.event_id = ?1
          ORDER BY s.channel_id, s.block_index",
    )?;
    let rows = stmt
        .query_map(params![event_id], |r| {
            Ok(SampleBlockRef {
                channel_path: r.get::<_, String>(0)?,
                t0_ms: r.get::<_, i64>(1)?,
                t1_ms: r.get::<_, i64>(2)?,
                sample_count: r.get::<_, i32>(3)? as i32,
                encoding: r.get::<_, i32>(4)? as i32,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Decode + concatenate all sample blocks on `(event_ulid, channel_path)`,
/// optionally constrained to `[from_ms, to_ms]`.
///
/// Returns the samples in time-ascending order (first by block t0, then by
/// in-block delta sequence). Empty result is returned for a missing event /
/// channel / block combination — callers wanting "no such channel" semantics
/// should use [`resolve_channel_and_event`] first.
pub fn read_samples_decoded(
    conn: &Connection,
    event_ulid: &Ulid,
    channel_path: &str,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
) -> Result<Vec<AbsoluteSample>> {
    let (event_id, channel_id) = resolve_channel_and_event(conn, event_ulid, channel_path)?;
    let mut stmt = conn.prepare(
        "SELECT t0_ms, t1_ms, sample_count, encoding, data
           FROM event_samples
          WHERE event_id = ?1 AND channel_id = ?2
          ORDER BY block_index",
    )?;
    let rows = stmt
        .query_map(params![event_id, channel_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i32>(2)?,
                r.get::<_, i32>(3)?,
                r.get::<_, Vec<u8>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out: Vec<AbsoluteSample> = Vec::new();
    for (t0, t1, _count, encoding, data) in rows {
        // Cheap range trim: skip blocks fully outside the window.
        if let Some(t_to) = to_ms {
            if t0 > t_to {
                continue;
            }
        }
        if let Some(t_from) = from_ms {
            if t1 < t_from {
                continue;
            }
        }
        let decoded = sample_codec::decode(encoding, &data)?;
        for s in decoded {
            let abs = AbsoluteSample {
                t_ms: t0 + s.t_offset_ms,
                value: s.value,
            };
            if let Some(t_from) = from_ms {
                if abs.t_ms < t_from {
                    continue;
                }
            }
            if let Some(t_to) = to_ms {
                if abs.t_ms > t_to {
                    continue;
                }
            }
            out.push(abs);
        }
    }
    Ok(out)
}

/// Resolve `(event_id, channel_id)` for a given wire ULID + channel path.
/// Errors with [`Error::NotFound`] if the event doesn't exist; with
/// [`Error::UnknownChannel`] if the channel isn't on the event's type.
pub fn resolve_channel_and_event(
    conn: &Connection,
    event_ulid: &Ulid,
    channel_path: &str,
) -> Result<(i64, i64)> {
    let rand_tail = ulid::random_tail(event_ulid);
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, event_type_id FROM events WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (event_id, et_id) = row.ok_or(Error::NotFound)?;
    let chan = registry::resolve_channel(conn, et_id, channel_path).map_err(|_| {
        let etype = registry::event_type_by_id(conn, et_id).ok();
        let etype_name = etype
            .map(|e| format!("{}.{}", e.namespace, e.name))
            .unwrap_or_else(|| format!("event_type_id={et_id}"));
        Error::UnknownChannel {
            event_type: etype_name,
            channel_path: channel_path.into(),
        }
    })?;
    Ok((event_id, chan.id))
}

/// Convenience: downsample by emitting at most `max` samples evenly spaced.
/// Used by `ReadSamplesRequest.max_samples` when non-zero.
pub fn downsample(samples: Vec<AbsoluteSample>, max: usize) -> Vec<AbsoluteSample> {
    if max == 0 || samples.len() <= max {
        return samples;
    }
    let step = samples.len() as f64 / max as f64;
    let mut out = Vec::with_capacity(max);
    let mut i = 0.0;
    while (i as usize) < samples.len() && out.len() < max {
        out.push(samples[i as usize]);
        i += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample_codec::{encode_f32, Sample};

    #[test]
    fn downsample_preserves_first_sample() {
        let s: Vec<AbsoluteSample> = (0..1000)
            .map(|i| AbsoluteSample {
                t_ms: i as i64,
                value: i as f64,
            })
            .collect();
        let d = downsample(s.clone(), 100);
        assert_eq!(d.len(), 100);
        assert_eq!(d[0].t_ms, 0);
    }

    #[test]
    fn downsample_no_op() {
        let s = vec![AbsoluteSample {
            t_ms: 0,
            value: 1.0,
        }];
        let d = downsample(s.clone(), 100);
        assert_eq!(d, s);
    }

    #[test]
    fn encode_compatibility() {
        // Sanity: the codec round-trips and the data is parseable by the decoder.
        let samples = vec![
            Sample {
                t_offset_ms: 0,
                value: 1.0,
            },
            Sample {
                t_offset_ms: 100,
                value: 2.0,
            },
        ];
        let bytes = encode_f32(&samples).unwrap();
        let decoded = sample_codec::decode(sample_codec::ENCODING_F32, &bytes).unwrap();
        assert_eq!(decoded.len(), 2);
    }
}
