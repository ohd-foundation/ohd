//! Shared helpers for the `profile.*` persistent-fact tools.
//!
//! Persistent facts (allergies, conditions, blood type, emergency
//! contacts, advance directives) are modelled as typed per-fact events:
//! each edit writes a fresh event carrying a stable `fact_id` and a
//! `status`. "Current state" is the projection *latest event per
//! `fact_id`, drop the removed/resolved ones* — computed here in Rust
//! over a small, event-type-indexed result set (never a full-log scan).
//!
//! This is the read-side counterpart to the `put` helpers: every
//! `list_*` tool and `get_health_profile` funnels through
//! [`active_facts`].

use crate::event_json::ms_to_iso;
use crate::ToolResult;
use ohd_storage_core::events::{
    query_events, ChannelScalar, Event, EventFilter, EventVisibility,
};
use ohd_storage_core::Storage;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Query every event of `event_type`, all visibilities, no time window.
/// Profile streams are tiny (a handful of rows) and event-type-indexed,
/// so the unbounded query is cheap.
pub fn query_all(storage: &Storage, event_type: &str) -> ToolResult<Vec<Event>> {
    let (events, _) = storage.with_conn(|conn| {
        query_events(
            conn,
            &EventFilter {
                from_ms: None,
                to_ms: None,
                event_types_in: vec![event_type.to_string()],
                limit: Some(10_000),
                visibility: EventVisibility::All,
                ..Default::default()
            },
            None,
        )
    })?;
    Ok(events)
}

/// Read a text channel off an event by path.
pub fn channel_text(e: &Event, path: &str) -> Option<String> {
    e.channels.iter().find(|c| c.channel_path == path).and_then(|c| match &c.value {
        ChannelScalar::Text { text_value } => Some(text_value.clone()),
        _ => None,
    })
}

/// Keep only the latest event per `fact_id`. Relies on `query_all`
/// returning newest-first (storage orders `timestamp_ms DESC, id DESC`,
/// so the first event seen for a `fact_id` is the most recent — the
/// random ULID tail is NOT a reliable recency signal, the rowid is).
/// Events without a `fact_id` channel are skipped.
fn latest_per_fact_id(events: &[Event]) -> Vec<&Event> {
    let mut seen: HashMap<String, &Event> = HashMap::new();
    for e in events {
        let Some(fid) = channel_text(e, "fact_id") else { continue };
        seen.entry(fid).or_insert(e);
    }
    seen.into_values().collect()
}

/// The "current" facts of a type: latest-per-fact-id, with any event
/// whose `status` is in `inactive_statuses` dropped. Returns each as the
/// flat JSON the tools surface (fact_id + ulid + ts + all channels).
pub fn active_facts(
    storage: &Storage,
    event_type: &str,
    inactive_statuses: &[&str],
) -> ToolResult<Vec<Value>> {
    let events = query_all(storage, event_type)?;
    let mut out: Vec<Value> = latest_per_fact_id(&events)
        .into_iter()
        .filter(|e| {
            channel_text(e, "status")
                .map(|s| !inactive_statuses.contains(&s.as_str()))
                .unwrap_or(true)
        })
        .map(flatten_event)
        .collect();
    // Stable, human-friendly order: newest first.
    out.sort_by(|a, b| {
        b.get("ts_ms").and_then(Value::as_i64).cmp(&a.get("ts_ms").and_then(Value::as_i64))
    });
    Ok(out)
}

/// Flatten one event to JSON: every channel hoisted to a top-level key,
/// plus ulid + ts. Shared by the profile-fact list tools and the
/// regimen projection.
pub fn flatten_event(e: &Event) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("ulid".into(), json!(e.ulid));
    map.insert("ts_ms".into(), json!(e.timestamp_ms));
    map.insert("ts_iso".into(), json!(ms_to_iso(e.timestamp_ms)));
    for c in &e.channels {
        map.insert(c.channel_path.clone(), crate::event_json::scalar_to_json(&c.value));
    }
    Value::Object(map)
}

/// Slug for a stable `fact_id` derived from a human label (e.g. the
/// allergen name). Lowercase, non-alphanumerics → `_`, collapse repeats.
/// A caller may pass an explicit `fact_id` to override this.
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_us = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    out.trim_matches('_').to_string()
}
