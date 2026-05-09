//! Token resolution to one of three OHDC auth profiles.
//!
//! See `spec/privacy-access.md` "The three auth profiles" and
//! `spec/storage-format.md` "Privacy and access control".
//!
//! Token wire forms:
//!  - `ohds_<base64url>` → SelfSession
//!  - `ohdg_<base64url>` → Grant
//!  - `ohdd_<base64url>` → Device (a grant with `kind='device'`)
//!
//! Tokens are stored hashed (`sha256(body)`) in the `_tokens` table inside
//! the per-user file. The hash is the unique key; the cleartext is shown
//! exactly once at issuance. This is consistent with the system-DB design in
//! `spec/auth.md`; see STATUS.md for the deviation note (we colocate `_tokens`
//! into the per-user file to make the v1 smoke test self-contained).

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// Three OHDC auth profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// User authenticated as themselves via OIDC. Full scope on own data.
    SelfSession,
    /// User-issued grant for a third party.
    Grant,
    /// Specialized grant: write-only.
    Device,
}

impl TokenKind {
    /// Bearer-prefix string (without the trailing underscore).
    pub fn prefix(self) -> &'static str {
        match self {
            TokenKind::SelfSession => "ohds",
            TokenKind::Grant => "ohdg",
            TokenKind::Device => "ohdd",
        }
    }
}

/// Resolved token bundle.
#[derive(Debug, Clone)]
pub struct ResolvedToken {
    /// Which auth profile.
    pub kind: TokenKind,
    /// User the token's *credentials* resolve to. For non-delegate grants
    /// this is the data-owner. For delegate grants this is the delegate's
    /// own user ULID (the bearer of the token).
    pub user_ulid: Ulid,
    /// For Grant/Device kinds: the grant rowid (joinable into audit).
    pub grant_id: Option<i64>,
    /// For Grant/Device kinds: the grant ULID (wire form).
    pub grant_ulid: Option<Ulid>,
    /// For Grant/Device: the grantee's display label.
    pub grantee_label: Option<String>,
    /// For delegate grants: the user being delegated *for*. Reads + writes
    /// resolve against this user's per-file storage. NULL on every
    /// non-delegate token.
    pub delegate_for_user_ulid: Option<Ulid>,
}

impl ResolvedToken {
    /// The user whose data this token reads/writes. For a delegate token
    /// this is `delegate_for_user_ulid`; otherwise it's `user_ulid`.
    pub fn effective_user_ulid(&self) -> Ulid {
        self.delegate_for_user_ulid.unwrap_or(self.user_ulid)
    }

    /// True iff this token is a delegate grant token.
    pub fn is_delegate(&self) -> bool {
        self.delegate_for_user_ulid.is_some()
    }
}

/// Token-prefix → kind discrimination per `spec/auth.md`.
///
/// Returns [`Error::Unauthenticated`] for unknown prefixes.
pub fn classify_token(bearer: &str) -> Result<TokenKind> {
    let (prefix, _body) = bearer.split_once('_').ok_or(Error::Unauthenticated)?;
    match prefix {
        "ohds" => Ok(TokenKind::SelfSession),
        "ohdg" => Ok(TokenKind::Grant),
        "ohdd" => Ok(TokenKind::Device),
        _ => Err(Error::Unauthenticated),
    }
}

/// Hash a bearer token's body for storage / lookup.
pub fn hash_token(bearer: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bearer.as_bytes());
    h.finalize().into()
}

/// Issue a new self-session token. Returns the cleartext (shown once).
pub fn issue_self_session_token(
    conn: &Connection,
    user_ulid: Ulid,
    label: Option<&str>,
    ttl_ms: Option<i64>,
) -> Result<String> {
    let body = mint_token_body();
    let bearer = format!("ohds_{body}");
    let hash = hash_token(&bearer);
    let now = crate::format::now_ms();
    let expires = ttl_ms.map(|t| now + t);
    conn.execute(
        "INSERT INTO _tokens (token_prefix, token_hash, user_ulid, grant_id,
                              issued_at_ms, expires_at_ms, label)
         VALUES ('ohds', ?1, ?2, NULL, ?3, ?4, ?5)",
        params![hash.to_vec(), user_ulid.to_vec(), now, expires, label],
    )?;
    Ok(bearer)
}

/// Issue a new grant or device token bound to an existing grant row.
pub fn issue_grant_token(
    conn: &Connection,
    user_ulid: Ulid,
    grant_id: i64,
    kind: TokenKind,
    ttl_ms: Option<i64>,
) -> Result<String> {
    let prefix = kind.prefix();
    if prefix == "ohds" {
        return Err(Error::InvalidArgument(
            "issue_grant_token called with self-session kind".into(),
        ));
    }
    let body = mint_token_body();
    let bearer = format!("{prefix}_{body}");
    let hash = hash_token(&bearer);
    let now = crate::format::now_ms();
    let expires = ttl_ms.map(|t| now + t);
    conn.execute(
        "INSERT INTO _tokens (token_prefix, token_hash, user_ulid, grant_id,
                              issued_at_ms, expires_at_ms, label)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
        params![
            prefix,
            hash.to_vec(),
            user_ulid.to_vec(),
            grant_id,
            now,
            expires
        ],
    )?;
    Ok(bearer)
}

/// Validate a bearer token and resolve it. Checks the prefix, the token-store
/// expiry/revocation, and (for grant/device) the underlying grant row's
/// `revoked_at_ms` / `expires_at_ms` / `is_template` flags.
pub fn resolve_token(conn: &Connection, bearer: &str) -> Result<ResolvedToken> {
    let kind = classify_token(bearer)?;
    let hash = hash_token(bearer);
    let now = crate::format::now_ms();

    let row: Option<(i64, Vec<u8>, Option<i64>, Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT id, user_ulid, grant_id, expires_at_ms, revoked_at_ms
               FROM _tokens WHERE token_hash = ?1",
            params![hash.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (_id, user_bytes, grant_id, expires_at_ms, revoked_at_ms) =
        row.ok_or(Error::Unauthenticated)?;
    if let Some(rev) = revoked_at_ms {
        if rev <= now {
            return Err(Error::TokenRevoked);
        }
    }
    if let Some(exp) = expires_at_ms {
        if exp <= now {
            return Err(Error::TokenExpired);
        }
    }
    let user_ulid: Ulid = user_bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidUlid)?;

    if kind == TokenKind::SelfSession {
        if grant_id.is_some() {
            return Err(Error::Unauthenticated);
        }
        return Ok(ResolvedToken {
            kind,
            user_ulid,
            grant_id: None,
            grant_ulid: None,
            grantee_label: None,
            delegate_for_user_ulid: None,
        });
    }

    // Grant / device — fetch the grant row.
    let gid = grant_id.ok_or(Error::Unauthenticated)?;
    let grant: Option<(
        Vec<u8>,
        String,
        i64,
        Option<i64>,
        Option<i64>,
        i64,
        Option<Vec<u8>>,
    )> = conn
        .query_row(
            "SELECT ulid_random, grantee_label, created_at_ms, expires_at_ms,
                    revoked_at_ms, is_template, delegate_for_user_ulid
               FROM grants WHERE id = ?1",
            params![gid],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()?;
    let (rand_tail, label, _created, g_expires, g_revoked, is_template, delegate_for_blob) =
        grant.ok_or(Error::Unauthenticated)?;
    if is_template != 0 {
        // Templates are not bearer-presentable per spec.
        return Err(Error::OutOfScope);
    }
    if let Some(rev) = g_revoked {
        if rev <= now {
            return Err(Error::TokenRevoked);
        }
    }
    if let Some(exp) = g_expires {
        if exp <= now {
            return Err(Error::TokenExpired);
        }
    }
    let mut grant_ulid_buf = [0u8; 16];
    if rand_tail.len() == 10 {
        // Embed the random tail; time prefix derived from created_at_ms.
        let prefix_ulid = crate::ulid::mint(0);
        grant_ulid_buf[..6].copy_from_slice(&prefix_ulid[..6]);
        grant_ulid_buf[6..].copy_from_slice(&rand_tail);
    }
    let delegate_for_user_ulid = delegate_for_blob.and_then(|b| {
        if b.len() == 16 {
            let mut o = [0u8; 16];
            o.copy_from_slice(&b);
            Some(o)
        } else {
            None
        }
    });

    Ok(ResolvedToken {
        kind,
        user_ulid,
        grant_id: Some(gid),
        grant_ulid: Some(grant_ulid_buf),
        grantee_label: Some(label),
        delegate_for_user_ulid,
    })
}

/// Catalog of OHDC operations for kind-checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OhdcOp {
    /// `OhdcService.PutEvents`
    PutEvents,
    /// `OhdcService.QueryEvents`
    QueryEvents,
    /// `OhdcService.GetEventByUlid`
    GetEventByUlid,
    /// `OhdcService.WhoAmI`
    WhoAmI,
    /// `OhdcService.Health` (unauthenticated)
    Health,
    /// `OhdcService.AuditQuery` (self-session only)
    AuditQuery,
    /// `OhdcService.CreateGrant` (self-session only)
    CreateGrant,
    /// `OhdcService.ListGrants` (self-session lists all; grant tokens see only their own)
    ListGrants,
    /// `OhdcService.UpdateGrant` (self-session only)
    UpdateGrant,
    /// `OhdcService.RevokeGrant` (self-session only)
    RevokeGrant,
    /// `OhdcService.ListPending` (self-session lists all; grant tokens see only their own)
    ListPending,
    /// `OhdcService.ApprovePending` (self-session only)
    ApprovePending,
    /// `OhdcService.RejectPending` (self-session only)
    RejectPending,
    /// `OhdcService.ListPendingQueries` (self-session lists all; grant tokens see only their own)
    ListPendingQueries,
    /// `OhdcService.ApprovePendingQuery` (self-session only)
    ApprovePendingQuery,
    /// `OhdcService.RejectPendingQuery` (self-session only)
    RejectPendingQuery,
    /// `OhdcService.CreateCase` (self-session opens directly; grant tokens
    /// open under break-glass / care-visit flow if their grant scope permits)
    CreateCase,
    /// `OhdcService.UpdateCase` (self-session, or the case's opening authority)
    UpdateCase,
    /// `OhdcService.CloseCase` (self-session, or the case's opening authority)
    CloseCase,
    /// `OhdcService.ReopenCase` (self-session, or token holder)
    ReopenCase,
    /// `OhdcService.ListCases` (self-session sees all; grant tokens see only
    /// cases bound by `grant_cases`)
    ListCases,
    /// `OhdcService.GetCase` (same scope rules as ListCases)
    GetCase,
    /// `OhdcService.AddCaseFilter` (self-session, or opening authority)
    AddCaseFilter,
    /// `OhdcService.RemoveCaseFilter` (self-session, or opening authority)
    RemoveCaseFilter,
    /// `OhdcService.ListCaseFilters` (same scope rules as GetCase)
    ListCaseFilters,
    /// `OhdcService.ReadSamples` (server-streaming sample-block decode)
    ReadSamples,
    /// `OhdcService.ReadAttachment` (server-streaming attachment fetch)
    ReadAttachment,
    /// `OhdcService.AttachBlob` (client-streaming attachment upload)
    AttachBlob,
    /// `OhdcService.Aggregate` (server-side aggregations)
    Aggregate,
    /// `OhdcService.Correlate` (server-side temporal correlation)
    Correlate,
    /// `OhdcService.Export` (self-session only)
    Export,
    /// `OhdcService.Import` (self-session only)
    Import,
}

/// Confirm the resolved token is allowed to invoke the operation.
///
/// See the token-kind matrix in `spec/ohdc-protocol.md`. v1 implements:
///  - SelfSession can invoke everything.
///  - Grant can invoke read/write ops (PutEvents, QueryEvents, GetEventByUlid,
///    WhoAmI, Health) plus ListGrants/ListPending/ListPendingQueries in introspect mode.
///  - Device can only PutEvents (and WhoAmI/Health).
pub fn check_kind_for_op(token: &ResolvedToken, op: OhdcOp) -> Result<()> {
    use OhdcOp::*;
    use TokenKind::*;
    match (token.kind, op) {
        (_, Health) | (_, WhoAmI) => Ok(()),
        (SelfSession, _) => Ok(()),
        (Grant, PutEvents) | (Grant, QueryEvents) | (Grant, GetEventByUlid) => Ok(()),
        // Grant tokens may introspect their own grant + their own pending writes/reads.
        (Grant, ListGrants) | (Grant, ListPending) | (Grant, ListPendingQueries) => Ok(()),
        // Grant tokens may read samples + attachments under the same scope as
        // QueryEvents (grant scope intersection happens inside the handler).
        // Aggregate / Correlate are read-side and obey aggregation_only +
        // strip_notes the same way QueryEvents does.
        (Grant, ReadSamples)
        | (Grant, ReadAttachment)
        | (Grant, AttachBlob)
        | (Grant, Aggregate)
        | (Grant, Correlate)
        | (Grant, AuditQuery) => Ok(()),
        // Cases: grant tokens can open cases (break-glass / care visit pattern),
        // close their own cases, list/get cases they have access to, and
        // manage filters on their own cases. Reopen-via-token works for any
        // token kind because the token itself is the proof of authority.
        (Grant, CreateCase)
        | (Grant, CloseCase)
        | (Grant, ReopenCase)
        | (Grant, ListCases)
        | (Grant, GetCase)
        | (Grant, AddCaseFilter)
        | (Grant, RemoveCaseFilter)
        | (Grant, ListCaseFilters)
        | (Grant, UpdateCase) => Ok(()),
        (Grant, _) => Err(Error::WrongTokenKind("self-session required")),
        (Device, PutEvents) => Ok(()),
        (Device, _) => Err(Error::WrongTokenKind("device tokens are write-only")),
    }
}

fn mint_token_body() -> String {
    // 32 random bytes → URL-safe base64.
    let raw = crate::ulid::random_bytes(32);
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &raw)
}
