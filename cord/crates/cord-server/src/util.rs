//! Small shared helpers.

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Current UTC time as an RFC 3339 string — the storage form for every
/// timestamp column.
pub fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// A fresh ULID string — used for every primary key CORD mints.
pub fn new_ulid() -> String {
    ulid::Ulid::new().to_string()
}
