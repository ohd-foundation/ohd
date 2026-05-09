//! Error types for the storage core.
//!
//! Maps onto the OHDC error catalog (see `spec/ohdc-protocol.md` "Error
//! model"). The transport layer translates these into HTTP status codes and
//! `google.rpc.Status` bodies.

use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Storage-core error.
#[derive(Debug, Error)]
pub enum Error {
    /// Stub: function is part of the v0 scaffold and not implemented yet.
    #[error("not implemented (v0 scaffold): {0}")]
    NotImplemented(&'static str),

    // --- Validation (registry / structural) ---
    /// `event_type` not in registry.
    #[error("unknown event_type: {0}")]
    UnknownType(String),
    /// `channel_path` not in registry for the given type.
    #[error("unknown channel: {event_type}/{channel_path}")]
    UnknownChannel {
        /// Event type the channel was looked up against.
        event_type: String,
        /// Channel path that didn't resolve.
        channel_path: String,
    },
    /// Channel value oneof mismatches the channel's declared `value_type`.
    #[error("wrong value type for channel {0}")]
    WrongValueType(String),
    /// Submission specified a non-canonical unit.
    #[error("invalid unit for channel {0}")]
    InvalidUnit(String),
    /// Enum ordinal out of range.
    #[error("invalid enum ordinal for channel {0}")]
    InvalidEnum(String),
    /// Required channel absent and its parent group present.
    #[error("missing required channel {0}")]
    MissingRequiredChannel(String),
    /// ULID byte length or format wrong.
    #[error("invalid ULID")]
    InvalidUlid,
    /// Timestamp outside acceptable range or wire-decode failure.
    #[error("invalid timestamp")]
    InvalidTimestamp,
    /// Filter expression unparseable or references unknown fields.
    #[error("invalid filter: {0}")]
    InvalidFilter(String),
    /// Generic argument validation failure.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    // --- Auth / authz ---
    /// Bearer token missing or unrecognized.
    #[error("unauthenticated")]
    Unauthenticated,
    /// Token recognized but past expiry.
    #[error("token expired")]
    TokenExpired,
    /// Token recognized but revoked.
    #[error("token revoked")]
    TokenRevoked,
    /// Operation requires a different token kind than the one provided.
    #[error("wrong token kind: {0}")]
    WrongTokenKind(&'static str),
    /// Token is valid but doesn't grant the requested operation or filter.
    #[error("out of scope")]
    OutOfScope,
    /// `require_approval_per_query`: the user didn't approve in time.
    #[error("approval timeout")]
    ApprovalTimeout,
    /// `require_approval_per_query`: the query is queued for user approval.
    /// Carries the query ULID + auto-expiry so the grantee can re-poll.
    #[error("pending approval (query_ulid={ulid_crockford})")]
    PendingApproval {
        /// Crockford-base32 ULID of the queued query.
        ulid_crockford: String,
        /// Auto-expiry of the pending row.
        expires_at_ms: i64,
    },

    // --- Lookup ---
    /// Resource not found (or out-of-scope for grant tokens — same wire code).
    #[error("not found")]
    NotFound,

    // --- Lifecycle ---
    /// Referenced event has `deleted_at_ms` set.
    #[error("event deleted")]
    EventDeleted,
    /// Grant exists but `revoked_at_ms` set.
    #[error("grant revoked")]
    GrantRevoked,
    /// Grant `expires_at_ms` past.
    #[error("grant expired")]
    GrantExpired,
    /// Case-bound grant whose case has closed.
    #[error("case closed")]
    CaseClosed,
    /// Case ULID referenced doesn't exist or out of scope.
    #[error("case not found")]
    CaseNotFound,
    /// `(source, source_id)` reused with different content.
    #[error("idempotency conflict")]
    IdempotencyConflict,

    // --- Encryption ---
    /// AES-GCM decryption failed (wrong key, tampered ciphertext, wrong AAD,
    /// or malformed nonce / blob length).
    ///
    /// Returned by the channel-encryption pipeline when a wrapped DEK can't
    /// be unwrapped under the supplied K_envelope, when a value blob's tag
    /// doesn't verify, or when an encrypted blob's `encryption_key_id`
    /// references a `class_key_history` row that no longer exists.
    #[error("decryption failed")]
    DecryptionFailed,

    // --- Resource limits ---
    /// Per-grant or per-user rate limit hit.
    #[error("rate limited")]
    RateLimited,
    /// Event batch, attachment, or sample block exceeds limits.
    #[error("payload too large")]
    PayloadTooLarge,
    /// Destination has no space (cache mode physical full).
    #[error("storage full")]
    StorageFull,

    // --- Format / version ---
    /// Client requested a version the storage doesn't support.
    #[error("unsupported protocol version: {0}")]
    UnsupportedProtocolVersion(String),
    /// Sample-block encoding ID unknown to this implementation.
    #[error("unsupported sample-block encoding: {0}")]
    UnsupportedEncoding(i32),

    // --- Catch-alls ---
    /// I/O error from the storage backend.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// SQLite engine error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON encode/decode error.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Anything else.
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),
}

impl Error {
    /// Map to the OHDC `ErrorInfo.code` string used over the wire.
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotImplemented(_) => "UNIMPLEMENTED",
            Error::UnknownType(_) => "UNKNOWN_TYPE",
            Error::UnknownChannel { .. } => "UNKNOWN_CHANNEL",
            Error::WrongValueType(_) => "WRONG_VALUE_TYPE",
            Error::InvalidUnit(_) => "INVALID_UNIT",
            Error::InvalidEnum(_) => "INVALID_ENUM",
            Error::MissingRequiredChannel(_) => "MISSING_REQUIRED_CHANNEL",
            Error::InvalidUlid => "INVALID_ULID",
            Error::InvalidTimestamp => "INVALID_TIMESTAMP",
            Error::InvalidFilter(_) => "INVALID_FILTER",
            Error::InvalidArgument(_) => "INVALID_ARGUMENT",
            Error::Unauthenticated => "UNAUTHENTICATED",
            Error::TokenExpired => "TOKEN_EXPIRED",
            Error::TokenRevoked => "TOKEN_REVOKED",
            Error::WrongTokenKind(_) => "WRONG_TOKEN_KIND",
            Error::OutOfScope => "OUT_OF_SCOPE",
            Error::ApprovalTimeout => "APPROVAL_TIMEOUT",
            Error::PendingApproval { .. } => "PENDING_APPROVAL",
            Error::NotFound => "NOT_FOUND",
            Error::EventDeleted => "EVENT_DELETED",
            Error::GrantRevoked => "GRANT_REVOKED",
            Error::GrantExpired => "GRANT_EXPIRED",
            Error::CaseClosed => "CASE_CLOSED",
            Error::CaseNotFound => "CASE_NOT_FOUND",
            Error::IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
            Error::RateLimited => "RATE_LIMITED",
            Error::DecryptionFailed => "DECRYPTION_FAILED",
            Error::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            Error::StorageFull => "STORAGE_FULL",
            Error::UnsupportedProtocolVersion(_) => "UNSUPPORTED_PROTOCOL_VERSION",
            Error::UnsupportedEncoding(_) => "UNSUPPORTED_ENCODING",
            Error::Io(_) => "IO_ERROR",
            Error::Sqlite(_) => "INTERNAL",
            Error::Json(_) => "INVALID_ARGUMENT",
            Error::Internal(_) => "INTERNAL",
        }
    }

    /// HTTP status code (for the OHDC HTTP/JSON wire envelope).
    pub fn http_status(&self) -> u16 {
        match self {
            Error::Unauthenticated | Error::TokenExpired | Error::TokenRevoked => 401,
            Error::WrongTokenKind(_) | Error::OutOfScope => 403,
            Error::NotFound | Error::CaseNotFound | Error::EventDeleted => 404,
            Error::IdempotencyConflict => 409,
            Error::RateLimited => 429,
            Error::DecryptionFailed => 500,
            Error::PayloadTooLarge => 413,
            Error::ApprovalTimeout => 408,
            Error::PendingApproval { .. } => 202,
            Error::UnknownType(_)
            | Error::UnknownChannel { .. }
            | Error::WrongValueType(_)
            | Error::InvalidUnit(_)
            | Error::InvalidEnum(_)
            | Error::MissingRequiredChannel(_)
            | Error::InvalidUlid
            | Error::InvalidTimestamp
            | Error::InvalidFilter(_)
            | Error::InvalidArgument(_)
            | Error::Json(_)
            | Error::UnsupportedEncoding(_)
            | Error::UnsupportedProtocolVersion(_) => 400,
            _ => 500,
        }
    }
}
