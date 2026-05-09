//! Time parsing for query commands.
//!
//! Accepts the `--last-day` / `--last-week` / `--last-month` shortcuts plus
//! `--from <ISO8601>` / `--to <ISO8601>`. Returns Unix milliseconds, which is
//! what OHDC's `EventFilter.from_ms` / `to_ms` expects (signed; pre-1970
//! supported).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, TimeZone, Utc};

/// Resolved (from_ms, to_ms) pair. Either side may be `None` (open range).
#[derive(Debug, Clone, Copy)]
pub struct Range {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub enum LastWindow {
    Day,
    Week,
    Month,
}

impl LastWindow {
    fn duration(self) -> Duration {
        match self {
            LastWindow::Day => Duration::days(1),
            LastWindow::Week => Duration::weeks(1),
            // Calendar months vary; we approximate as 30 days for the CLI
            // shortcut. Users who want exact calendar bounds use --from/--to.
            LastWindow::Month => Duration::days(30),
        }
    }
}

/// Build a `Range` from the (mutually exclusive in clap, but checked again
/// here) inputs. Empty inputs → fully-open range.
pub fn build_range(
    last: Option<LastWindow>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Range> {
    if last.is_some() && (from.is_some() || to.is_some()) {
        return Err(anyhow!(
            "--last-* is mutually exclusive with --from / --to"
        ));
    }

    if let Some(window) = last {
        let now: DateTime<Utc> = Utc::now();
        let then = now - window.duration();
        return Ok(Range {
            from_ms: Some(then.timestamp_millis()),
            to_ms: None,
        });
    }

    let from_ms = from.map(parse_iso).transpose()?;
    let to_ms = to.map(parse_iso).transpose()?;
    Ok(Range { from_ms, to_ms })
}

/// Parse an ISO8601 / RFC3339 timestamp into Unix ms. Accepts both
/// `2026-01-02T03:04:05Z` and `2026-01-02T03:04:05+02:00`. Bare dates
/// (`2026-01-02`) are interpreted as midnight UTC for convenience.
pub fn parse_iso(s: &str) -> Result<i64> {
    // Try full RFC3339 first.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }
    // Fall back to a bare date (midnight UTC).
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
        return Ok(dt.timestamp_millis());
    }
    Err(anyhow!(
        "could not parse {s:?} as ISO8601 (expected e.g. 2026-01-02 or 2026-01-02T03:04:05Z)"
    ))
}

/// Render a Unix-ms timestamp as RFC3339 (UTC) for display in the query
/// table. Prints `?` if the conversion ever fails (out-of-range nanoseconds).
pub fn render_ms(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "?".to_string())
}
