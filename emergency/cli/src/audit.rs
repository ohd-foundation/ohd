//! `audit` subcommand bodies.
//!
//! Wraps `OhdcService.AuditQuery`. The storage server returns
//! `Unimplemented` for AuditQuery today (see
//! `../../storage/STATUS.md` "8. AuditQuery server-streaming handler …"),
//! so the CLI surfaces that cleanly: the network call still runs and
//! whatever the server returns is reported. Once the RPC lands the CLI
//! transparently starts producing real output.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::StreamExt;

use crate::client::OhdcClient;
use crate::proto::ohdc::v0 as pb;
use ohd_cli_auth::ulid;

pub struct ListArgs<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub responder: Option<&'a str>,
}

pub async fn cmd_list(client: &OhdcClient, args: ListArgs<'_>) -> Result<()> {
    let entries = run_query(client, &args).await?;
    if entries.is_empty() {
        eprintln!("(no audit entries)");
        return Ok(());
    }
    println!(
        "{:<25}  {:<10}  {:<14}  {:<10}  {:<8}  {}",
        "TIMESTAMP (UTC)", "ACTOR", "ACTION", "QUERY-KIND", "RESULT", "GRANT"
    );
    for e in &entries {
        let grant = e
            .grant_ulid
            .as_option()
            .map(|u| ulid::render_ulid_bytes(&u.bytes))
            .unwrap_or_else(|| "-".into());
        let ts = ms_to_rfc3339(e.ts_ms);
        println!(
            "{:<25}  {:<10}  {:<14}  {:<10}  {:<8}  {}",
            ts, e.actor_type, e.action, e.query_kind, e.result, grant
        );
    }
    eprintln!("({} entries)", entries.len());
    Ok(())
}

pub async fn cmd_export(
    client: &OhdcClient,
    output: &Path,
    args: ListArgs<'_>,
) -> Result<()> {
    let entries = run_query(client, &args).await?;
    let mut wtr = csv_writer(output)?;
    // Header
    wtr.write_record([
        "ts_ms",
        "ts_iso",
        "actor_type",
        "grant_ulid",
        "action",
        "query_kind",
        "query_params_json",
        "rows_returned",
        "rows_filtered",
        "result",
        "reason",
        "caller_ip",
        "caller_ua",
    ])?;
    for e in &entries {
        let grant_ulid = e
            .grant_ulid
            .as_option()
            .map(|u| ulid::render_ulid_bytes(&u.bytes))
            .unwrap_or_default();
        wtr.write_record([
            &e.ts_ms.to_string(),
            &ms_to_rfc3339(e.ts_ms),
            &e.actor_type,
            &grant_ulid,
            &e.action,
            &e.query_kind,
            &e.query_params_json,
            &e.rows_returned.map(|v| v.to_string()).unwrap_or_default(),
            &e.rows_filtered.map(|v| v.to_string()).unwrap_or_default(),
            &e.result,
            e.reason.as_deref().unwrap_or(""),
            e.caller_ip.as_deref().unwrap_or(""),
            e.caller_ua.as_deref().unwrap_or(""),
        ])?;
    }
    wtr.flush()?;
    println!(
        "wrote {} audit entr{} → {}",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
        output.display()
    );
    Ok(())
}

async fn run_query(client: &OhdcClient, args: &ListArgs<'_>) -> Result<Vec<pb::AuditEntry>> {
    let from_ms = match args.from {
        Some(s) => Some(parse_iso(s).context("--from")?),
        None => None,
    };
    let to_ms = match args.to {
        Some(s) => Some(parse_iso(s).context("--to")?),
        None => None,
    };
    let req = pb::AuditQueryRequest {
        from_ms,
        to_ms,
        ..Default::default()
    };

    // The server-side AuditQuery handler returns Unimplemented today; the
    // call still goes through (Connect framing on the wire, gRPC trailer
    // with code "unimplemented" coming back), and we surface the error
    // verbatim. Once the handler lands the same call wires up unchanged.
    let mut stream = client.audit_query(req).await.with_context(|| {
        "AuditQuery RPC failed. The storage server may still return Unimplemented \
         for AuditQuery (see ../../storage/STATUS.md). When the handler lands the \
         CLI transparently uses it."
    })?;

    // `--responder` filters client-side (the wire `AuditQueryRequest` has
    // optional `actor_type` / `grant_ulid` filters; "responder" is the
    // operator-side name and doesn't map 1:1 to a wire field today).
    let mut entries = Vec::new();
    while let Some(item) = stream.next().await {
        let entry = item?;
        if let Some(label) = args.responder {
            // Match against the embedded query_params_json (best-effort
            // until the wire schema grows a labelled responder field).
            if !entry.query_params_json.contains(label) {
                continue;
            }
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_iso(s: &str) -> Result<i64> {
    // Accept either a date (`2026-05-08`) or a full RFC 3339 timestamp.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc).timestamp_millis());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis());
    }
    Err(anyhow::anyhow!(
        "expected RFC 3339 timestamp or YYYY-MM-DD date, got {s:?}"
    ))
}

fn ms_to_rfc3339(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| format!("{ms}ms"))
}

// ---- minimal embedded CSV writer (no `csv` crate dependency) ------------
//
// The audit export volume is bounded by retention policy (typically O(MB))
// and the output is plain RFC 4180 CSV; pulling the `csv` crate just for
// quoting is overkill. The writer below handles double-quote escaping and
// embedded newlines / commas.

struct CsvWriter {
    out: std::fs::File,
}

fn csv_writer(path: &Path) -> Result<CsvWriter> {
    let out = std::fs::File::create(path)
        .with_context(|| format!("create {}", path.display()))?;
    Ok(CsvWriter { out })
}

impl CsvWriter {
    fn write_record<I, S>(&mut self, fields: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        use std::io::Write;
        let mut first = true;
        for f in fields {
            if !first {
                self.out.write_all(b",")?;
            }
            first = false;
            let s = f.as_ref();
            if s.contains([',', '"', '\n', '\r']) {
                self.out.write_all(b"\"")?;
                let escaped = s.replace('"', "\"\"");
                self.out.write_all(escaped.as_bytes())?;
                self.out.write_all(b"\"")?;
            } else {
                self.out.write_all(s.as_bytes())?;
            }
        }
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        use std::io::Write;
        self.out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_date_and_full() {
        // We don't hard-code the epoch; instead we assert the date and
        // the date-with-time differ by exactly the offset.
        let d = parse_iso("2026-05-08").unwrap();
        let f = parse_iso("2026-05-08T12:00:00Z").unwrap();
        assert_eq!(f - d, 12 * 3600 * 1000);
        // And spot-check the date is reasonable (year 2026 ≈ 1.77e12 ms).
        assert!(d > 1_700_000_000_000);
        assert!(d < 1_800_000_000_000);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_iso("not-a-date").is_err());
    }

    #[test]
    fn csv_round_trips_escaping() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.csv");
        let mut w = csv_writer(&p).unwrap();
        w.write_record(["plain", "with,comma", "with\"quote", "with\nnewline"])
            .unwrap();
        w.flush().unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        // `with,comma` → quoted; `with"quote` → escaped doubled quote;
        // newline → quoted.
        assert!(raw.contains("\"with,comma\""));
        assert!(raw.contains("\"with\"\"quote\""));
        assert!(raw.contains("\"with\nnewline\""));
    }
}
