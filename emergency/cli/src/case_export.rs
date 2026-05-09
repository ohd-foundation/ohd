//! `case-export` — build a portable archive of one OHD case for legal /
//! regulatory review.
//!
//! ## Archive format (v1)
//!
//! A single JSON file (`.json`). Atomic, human-readable, easy to attach
//! to a chain-of-custody envelope. The archive shape is pinned by
//! [`CaseArchive`]:
//!
//! ```jsonc
//! {
//!   "schema":         "ohd-emergency.case-export.v1",
//!   "exported_at_ms": 1778284800000,
//!   "exported_by":    "ohd-emergency 0.0.1",
//!   "exporter_label": "EMS Prague",        // from config.station_label
//!   "storage_url":    "http://localhost:8443",
//!   "case_ulid":      "01J...",            // Crockford-base32, 26 chars
//!   "case":   { ... pb::Case as proto3 JSON },
//!   "events": [ ... pb::Event as proto3 JSON ... ],
//!   "audit":  [ ... pb::AuditEntry as proto3 JSON ... ],
//!   "audit_status": "ok" | "rpc_unimplemented" | "rpc_error"
//! }
//! ```
//!
//! `audit_status` records why the audit array might be empty: storage's
//! `AuditQuery` is `Unimplemented` today (see ../../storage/STATUS.md), so
//! the archive carries a marker rather than silently producing empty
//! output.
//!
//! buffa 0.5 (with the `json` feature) emits `serde::Serialize` on the
//! generated proto types, so we lift each pb::* directly through
//! `serde_json::to_value` — no hand-rolled proto-to-JSON code needed.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use serde::Serialize;

use crate::client::OhdcClient;
use crate::config::Config;
use crate::proto::ohdc::v0 as pb;
use ohd_cli_auth::ulid;

/// Pinned schema identifier embedded in every archive.
pub const ARCHIVE_SCHEMA: &str = "ohd-emergency.case-export.v1";

#[derive(Debug, Serialize)]
pub struct CaseArchive {
    pub schema: &'static str,
    pub exported_at_ms: i64,
    pub exported_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exporter_label: Option<String>,
    pub storage_url: String,
    pub case_ulid: String,
    pub case: serde_json::Value,
    pub events: Vec<serde_json::Value>,
    pub audit: Vec<serde_json::Value>,
    pub audit_status: AuditStatus,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    /// Audit fetch succeeded.
    Ok,
    /// AuditQuery RPC returned Unimplemented (current storage state).
    RpcUnimplemented,
    /// AuditQuery RPC returned some other error.
    RpcError,
}

pub async fn cmd_case_export(
    client: &OhdcClient,
    cfg: Option<&Config>,
    case_ulid_str: &str,
    output: &Path,
) -> Result<()> {
    // 1. Resolve the case ULID.
    let case_bytes = ulid::parse_crockford(case_ulid_str)
        .with_context(|| format!("invalid case ULID {case_ulid_str:?}"))?;
    let case_ulid_proto = pb::Ulid {
        bytes: case_bytes.to_vec().into(),
        ..Default::default()
    };

    // 2. Fetch case metadata.
    let case = client
        .get_case(pb::GetCaseRequest {
            case_ulid: buffa::MessageField::some(case_ulid_proto.clone()),
            ..Default::default()
        })
        .await
        .with_context(|| {
            format!(
                "GetCase({case_ulid_str}) failed. Storage's GetCase handler may still \
                 return Unimplemented; see ../../storage/STATUS.md."
            )
        })?;

    // 3. Fetch case events. The wire `EventFilter` doesn't (yet) carry a
    //    case-bound predicate; we ask for all events on whatever scope the
    //    operator's token has. The case ULID is also recorded in the
    //    archive header, so the audience can scope manually. Once
    //    `EventFilter::case_ulids_in` lands this becomes a server-side
    //    filter.
    let events = fetch_events(client).await.context("fetch case events")?;

    // 4. Fetch audit entries (best-effort; AuditQuery is Unimplemented
    //    today, so capture and continue).
    let (audit, audit_status) = fetch_audit(client).await;

    // 5. Build the archive document.
    let archive = CaseArchive {
        schema: ARCHIVE_SCHEMA,
        exported_at_ms: Utc::now().timestamp_millis(),
        exported_by: format!("ohd-emergency {}", env!("CARGO_PKG_VERSION")),
        exporter_label: cfg.and_then(|c| c.station_label.clone()),
        storage_url: client.storage_url.clone(),
        case_ulid: case_ulid_str.to_string(),
        case: serde_json::to_value(&case).context("serialize case to JSON")?,
        events: events
            .iter()
            .map(|e| serde_json::to_value(e).context("serialize event"))
            .collect::<Result<Vec<_>>>()?,
        audit: audit
            .iter()
            .map(|a| serde_json::to_value(a).context("serialize audit entry"))
            .collect::<Result<Vec<_>>>()?,
        audit_status,
    };

    // 6. Write atomically (temp file + rename).
    let tmp = output.with_extension("json.tmp");
    let f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    serde_json::to_writer_pretty(&f, &archive).context("write JSON archive")?;
    drop(f);
    std::fs::rename(&tmp, output)
        .with_context(|| format!("rename {} → {}", tmp.display(), output.display()))?;

    println!(
        "wrote case-export archive (case={}, events={}, audit={}) → {}",
        case_ulid_str,
        archive.events.len(),
        match archive.audit_status {
            AuditStatus::Ok => format!("{}", archive.audit.len()),
            AuditStatus::RpcUnimplemented => "n/a (RPC unimplemented)".to_string(),
            AuditStatus::RpcError => "n/a (RPC error)".to_string(),
        },
        output.display()
    );
    Ok(())
}

async fn fetch_events(client: &OhdcClient) -> Result<Vec<pb::Event>> {
    let mut stream = client
        .query_events(pb::QueryEventsRequest {
            filter: buffa::MessageField::some(pb::EventFilter {
                include_superseded: true,
                limit: None,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item?);
    }
    Ok(out)
}

async fn fetch_audit(client: &OhdcClient) -> (Vec<pb::AuditEntry>, AuditStatus) {
    let req = pb::AuditQueryRequest::default();
    match client.audit_query(req).await {
        Ok(mut stream) => {
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                match item {
                    Ok(e) => out.push(e),
                    Err(_) => return (out, AuditStatus::RpcError),
                }
            }
            (out, AuditStatus::Ok)
        }
        Err(e) => {
            let msg = format!("{e:#}");
            if msg.to_lowercase().contains("unimplement") {
                (Vec::new(), AuditStatus::RpcUnimplemented)
            } else {
                (Vec::new(), AuditStatus::RpcError)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_serializes_through_buffa_serde() {
        // Smoke: confirm a buffa-emitted Case round-trips through
        // serde_json. If buffa drops the Serialize derive in a future
        // release this test catches it before users see broken archives.
        let mut c = pb::Case::default();
        c.case_type = "emergency".into();
        c.case_label = Some("test".into());
        c.started_at_ms = 1_700_000_000_000;
        c.last_activity_at_ms = 1_700_000_001_000;
        let v = serde_json::to_value(&c).unwrap();
        let map = v.as_object().expect("object");
        // buffa emits proto3 JSON `lowerCamelCase` (with `alias` for
        // `snake_case`); pinning the camelCase form here.
        assert_eq!(
            map.get("caseType").and_then(|x| x.as_str()),
            Some("emergency")
        );
    }

    #[test]
    fn archive_serializes() {
        let archive = CaseArchive {
            schema: ARCHIVE_SCHEMA,
            exported_at_ms: 1_700_000_000_000,
            exported_by: "ohd-emergency 0.0.1".into(),
            exporter_label: Some("EMS Test".into()),
            storage_url: "http://localhost:8443".into(),
            case_ulid: "0".repeat(26),
            case: serde_json::json!({"caseType": "emergency"}),
            events: vec![],
            audit: vec![],
            audit_status: AuditStatus::RpcUnimplemented,
        };
        let s = serde_json::to_string(&archive).unwrap();
        assert!(s.contains("ohd-emergency.case-export.v1"));
        assert!(s.contains("rpc_unimplemented"));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["schema"], "ohd-emergency.case-export.v1");
        assert_eq!(v["exporter_label"], "EMS Test");
        assert_eq!(v["case_ulid"].as_str().unwrap().len(), 26);
    }
}
