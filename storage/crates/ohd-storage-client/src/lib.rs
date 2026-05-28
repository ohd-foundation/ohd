//! OHD Storage remote client — the OHDC ConnectRPC client.
//!
//! Phase 1 of the Android remote-storage feature. This crate hosts the
//! generated `OhdcServiceClient` (a Tower-based ConnectRPC client, from the
//! `connectrpc` crate) and wraps it in [`OhdcRemoteClient`] — an
//! `async`, plain-Rust handle that talks to a remote `ohd-storage-server`
//! over Connect-RPC.
//!
//! # Transport choice
//!
//! `connectrpc 0.4` (already the workspace's server-side RPC stack) ships a
//! first-class **client** behind its `client` / `client-tls` features, and
//! the proto codegen (`connectrpc-build`) emits `OhdcServiceClient<T>` right
//! next to the server trait. We use it directly rather than hand-rolling
//! Connect framing over `reqwest`: the framing, the binary Protobuf codec,
//! the `application/connect+proto` content negotiation, the gRPC-status
//! trailer parsing, and the server-streaming decoders are all already
//! implemented, tested, and wire-compatible with the server this crate
//! talks to. Hand-rolling would re-implement exactly that surface with more
//! risk. The whole tree is pure-Rust / ring-backed, so it cross-compiles for
//! Android the same way `ohd-relay-client` does.
//!
//! # Surface
//!
//! [`OhdcRemoteClient`] mirrors the local `OhdStorage` read/write data ops
//! the OHD Connect app uses. It returns plain owned structs (this crate has
//! no uniffi dependency); `ohd-storage-bindings` maps those to the uniffi
//! DTOs so the Android `StorageRepository` sees the same shapes whether the
//! backend is local or remote.
//!
//! ULIDs cross this surface as raw 16-byte vectors — the uniffi layer owns
//! the Crockford-base32 rendering (it already links `ohd-storage-core`'s
//! `ulid` module).

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use connectrpc::client::{CallOptions, ClientConfig, HttpClient};
use connectrpc::Protocol;

/// The codegen-emitted OHDC protobuf message types + `OhdcServiceClient`.
///
/// Re-exported so integration tests (and any future in-workspace consumer)
/// can name the wire types, but [`OhdcRemoteClient`]'s own surface is
/// plain-Rust — callers never need to touch `pb` directly.
pub mod pb {
    connectrpc::include_generated!();
    pub use ohdc::v0::*;
}

mod convert;
mod types;

pub use types::*;

// =============================================================================
// Errors
// =============================================================================

/// Errors surfaced by [`OhdcRemoteClient`].
///
/// Mirrors the category split of the binding's `OhdError` so the uniffi
/// wrapper can map 1:1 without inventing new variants. `code` carries the
/// OHDC `ErrorInfo.code` string when the server supplied one.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    /// Transport failure — connection refused, TLS handshake failed, DNS,
    /// timeout. Retryable.
    #[error("transport error: {message}")]
    Transport {
        /// Human-readable detail.
        message: String,
    },
    /// Token missing, expired, revoked, wrong kind, or operation out of
    /// scope. The uniffi layer maps this to `OhdError::Auth`. When
    /// [`RemoteError::is_token_expired`] is true the Android layer should
    /// refresh the bearer token and retry.
    #[error("auth failed ({code}): {message}")]
    Auth {
        /// OHDC error code (e.g. `TOKEN_EXPIRED`, `WRONG_TOKEN_KIND`).
        code: String,
        /// Human-readable message.
        message: String,
    },
    /// Input validation failure — unknown event type, bad ULID, invalid
    /// filter, etc.
    #[error("invalid input ({code}): {message}")]
    InvalidInput {
        /// OHDC error code.
        code: String,
        /// Human-readable message.
        message: String,
    },
    /// Resource not found.
    #[error("not found")]
    NotFound,
    /// Anything else — server-internal failure, decode error, unimplemented.
    #[error("remote error ({code}): {message}")]
    Internal {
        /// OHDC error code.
        code: String,
        /// Human-readable message.
        message: String,
    },
}

impl RemoteError {
    /// True when the server rejected the call because the bearer token has
    /// expired. The Android `RemoteStorageBackend` uses this as the signal
    /// to mint a fresh self-session token, call
    /// [`OhdcRemoteClient::set_bearer_token`], and retry the RPC once.
    ///
    /// The OHDC error model surfaces an expired token as `Unauthenticated`
    /// carrying the `TOKEN_EXPIRED` code (see `spec/ohdc-protocol.md`); a
    /// revoked or wrong-kind token uses a different code, so refresh-and-retry
    /// is gated on the code, not just the gRPC status.
    pub fn is_token_expired(&self) -> bool {
        matches!(self, RemoteError::Auth { code, .. } if code == "TOKEN_EXPIRED")
    }
}

/// Translate a `connectrpc::ConnectError` into a [`RemoteError`].
///
/// The server's `error_to_connect` packs the OHDC code into the message as
/// `"<CODE>: <detail>"` (see `ohd-storage-server/src/server.rs`); we parse it
/// back out so the category + code survive the round trip.
fn map_connect_error(err: connectrpc::ConnectError) -> RemoteError {
    use connectrpc::ErrorCode;

    let raw: String = err.message.clone().unwrap_or_default();
    // Split a leading "CODE: detail" prefix when the server set one.
    let (code, message): (String, String) = match raw.split_once(": ") {
        Some((c, rest))
            if !c.is_empty()
                && c.chars()
                    .all(|ch: char| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit()) =>
        {
            (c.to_string(), rest.to_string())
        }
        _ => (String::new(), raw.clone()),
    };

    match err.code {
        ErrorCode::Unauthenticated | ErrorCode::PermissionDenied => RemoteError::Auth {
            code: if code.is_empty() {
                "UNAUTHENTICATED".to_string()
            } else {
                code
            },
            message,
        },
        ErrorCode::InvalidArgument | ErrorCode::OutOfRange | ErrorCode::FailedPrecondition => {
            RemoteError::InvalidInput {
                code: if code.is_empty() {
                    "INVALID_ARGUMENT".to_string()
                } else {
                    code
                },
                message,
            }
        }
        ErrorCode::NotFound => RemoteError::NotFound,
        ErrorCode::Unavailable | ErrorCode::DeadlineExceeded => {
            // Surface the real transport cause (TLS handshake / DNS / connect
            // refused / timeout) to logcat — the Android UI otherwise folds
            // every Unavailable into a single generic "unreachable" string.
            tracing::warn!(code = ?err.code, detail = %message, "remote RPC transport failure");
            RemoteError::Transport { message }
        }
        _ => RemoteError::Internal {
            code: if code.is_empty() {
                format!("{:?}", err.code)
            } else {
                code
            },
            message,
        },
    }
}

/// Result alias for the client surface.
pub type Result<T> = std::result::Result<T, RemoteError>;

// =============================================================================
// Client
// =============================================================================

type GeneratedClient = pb::OhdcServiceClient<HttpClient>;

/// Async ConnectRPC client for a remote `ohd-storage-server`'s `OhdcService`.
///
/// Cheap to clone — the underlying `HttpClient` is a pooled, `Clone`-able
/// hyper client. The bearer token lives behind a `Mutex` so
/// [`set_bearer_token`](Self::set_bearer_token) can swap in a refreshed token
/// without rebuilding the client (the Android layer does this on
/// `TOKEN_EXPIRED`).
#[derive(Clone)]
pub struct OhdcRemoteClient {
    client: Arc<GeneratedClient>,
    /// Current bearer token. Applied as `Authorization: Bearer <token>` on
    /// every RPC via per-call `CallOptions` (so a refreshed token takes
    /// effect immediately, without a new `ClientConfig`).
    bearer: Arc<Mutex<String>>,
}

impl OhdcRemoteClient {
    /// Build a client against `base_url` (e.g. `https://storage.example.com`
    /// or `http://127.0.0.1:18443` for plaintext dev / tests), authenticating
    /// with `bearer_token`.
    ///
    /// `https://` URLs use a rustls (ring) TLS connector with the platform
    /// root store via `webpki-roots` semantics inherited from hyper-rustls'
    /// `native` is **not** used — we install the process-default ring crypto
    /// provider and the standard Mozilla roots. `http://` URLs use a
    /// plaintext connector (dev / in-process tests only).
    pub fn new(base_url: &str, bearer_token: &str) -> Result<Self> {
        // Install the ring crypto provider once for the process. Ignted if a
        // provider is already installed (idempotent across multiple clients
        // or a co-resident rustls user).
        let _ = rustls::crypto::ring::default_provider().install_default();

        let uri: http::Uri = base_url.parse().map_err(|e| RemoteError::InvalidInput {
            code: "INVALID_ARGUMENT".to_string(),
            message: format!("invalid base_url {base_url:?}: {e}"),
        })?;

        let is_tls = uri.scheme_str() == Some("https");
        let transport = if is_tls {
            HttpClient::with_tls(default_tls_config())
        } else {
            HttpClient::plaintext()
        };

        // Connect protocol over HTTP/1.1 + HTTP/2 — the lower-friction path
        // for a mobile client behind arbitrary proxies (no h2c prior
        // knowledge required). The server speaks Connect, gRPC and gRPC-Web
        // on the same handlers.
        let config = ClientConfig::new(uri)
            .protocol(Protocol::Connect)
            .proto()
            .default_timeout(Duration::from_secs(30));

        Ok(Self {
            client: Arc::new(pb::OhdcServiceClient::new(transport, config)),
            bearer: Arc::new(Mutex::new(bearer_token.to_string())),
        })
    }

    /// Replace the bearer token used for subsequent RPCs. Lets the Android
    /// layer inject a refreshed self-session token after a `TOKEN_EXPIRED`
    /// failure without rebuilding the client.
    pub fn set_bearer_token(&self, token: &str) {
        if let Ok(mut guard) = self.bearer.lock() {
            *guard = token.to_string();
        }
    }

    /// Per-call options carrying the current `Authorization: Bearer` header.
    fn auth_options(&self) -> CallOptions {
        let token = self
            .bearer
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        CallOptions::default().with_header("authorization", format!("Bearer {token}"))
    }

    // -------------------------------------------------------------------------
    // Diagnostics
    // -------------------------------------------------------------------------

    /// `WhoAmI` — resolve the calling token's identity + kind.
    pub async fn whoami(&self) -> Result<WhoAmI> {
        let resp = self
            .client
            .who_am_i_with_options(pb::WhoAmIRequest::default(), self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(convert::whoami_from_pb(resp.into_owned()))
    }

    /// `Health` — server liveness probe. Unauthenticated on the server side,
    /// but the auth header is harmless.
    pub async fn health(&self) -> Result<Health> {
        let resp = self
            .client
            .health_with_options(pb::HealthRequest::default(), self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(convert::health_from_pb(resp.into_owned()))
    }

    // -------------------------------------------------------------------------
    // Events
    // -------------------------------------------------------------------------

    /// `PutEvents` for a single event — mirrors the local `put_event`.
    pub async fn put_event(&self, input: EventInput) -> Result<PutEventOutcome> {
        let results = self.put_events(vec![input], false).await?;
        results.into_iter().next().ok_or_else(|| RemoteError::Internal {
            code: "INTERNAL".to_string(),
            message: "PutEvents returned no results".to_string(),
        })
    }

    /// `PutEvents` for a batch — one RPC for the whole list, returning one
    /// outcome per input in order. `atomic = true` asks the server to commit
    /// all-or-nothing. This is the path bulk writers (Health Connect sync,
    /// importers, multi-event logs) should use to avoid one round-trip per
    /// event.
    pub async fn put_events(
        &self,
        inputs: Vec<EventInput>,
        atomic: bool,
    ) -> Result<Vec<PutEventOutcome>> {
        let req = pb::PutEventsRequest {
            events: inputs.into_iter().map(convert::event_input_to_pb).collect(),
            atomic,
            ..Default::default()
        };
        let resp = self
            .client
            .put_events_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        let owned = resp.into_owned();
        Ok(owned
            .results
            .into_iter()
            .map(convert::put_event_result_from_pb)
            .collect())
    }

    /// `ListTools` — fetch the agent tool catalog as a JSON string. Same
    /// shape the local uniffi `list_tools()` returns; CORD on a remote-
    /// storage device calls this so it can run tools against the cloud DB.
    pub async fn list_tools(&self) -> Result<String> {
        let req = pb::ListToolsRequest::default();
        let resp = self
            .client
            .list_tools_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(resp.into_owned().catalog_json)
    }

    /// `ExecuteTool` — dispatch one tool by name. Tool-domain errors are
    /// encoded as JSON inside the returned string; the RPC only fails for
    /// transport / authz issues.
    pub async fn execute_tool(&self, name: &str, input_json: &str) -> Result<String> {
        let req = pb::ExecuteToolRequest {
            name: name.to_string(),
            input_json: input_json.to_string(),
            ..Default::default()
        };
        let resp = self
            .client
            .execute_tool_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(resp.into_owned().output_json)
    }

    /// `DeleteEvents` — bulk hard-delete events matching `filter`. Empty
    /// filter wipes ALL events owned by the authenticated identity. Returns
    /// the number of `events` rows removed (cascaded channels not counted).
    /// Self-session only on the server side; grant tokens get
    /// `PermissionDenied`.
    pub async fn delete_events(&self, filter: DeleteFilter) -> Result<u64> {
        let req = pb::DeleteEventsRequest {
            from_ms: filter.from_ms,
            to_ms: filter.to_ms,
            event_types: filter.event_types,
            ..Default::default()
        };
        let resp = self
            .client
            .delete_events_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        let owned = resp.into_owned();
        Ok(owned.deleted_count.max(0) as u64)
    }

    /// `QueryEvents` (server-streaming) — collected into a `Vec` so the
    /// surface stays synchronous-shaped, exactly as the local `query_events`
    /// returns `Vec<Event>`.
    pub async fn query_events(&self, filter: EventFilter) -> Result<Vec<Event>> {
        let req = pb::QueryEventsRequest {
            filter: ::buffa::MessageField::some(convert::event_filter_to_pb(filter)),
            ..Default::default()
        };
        let mut stream = self
            .client
            .query_events_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        let mut out = Vec::new();
        while let Some(view) = stream.message().await.map_err(map_connect_error)? {
            out.push(convert::event_from_pb(view.to_owned_message()));
        }
        Ok(out)
    }

    /// Count events matching `filter`. The OHDC `OhdcService` has no
    /// dedicated count RPC, so this drains `QueryEvents` and counts the
    /// rows — the same observable result the local `count_events` produces,
    /// at the cost of materialising the stream. The Home stat tile is the
    /// `CountEvents` — pure SQL `COUNT(*)` on the server. Honours the same
    /// time / event-type / deleted predicates as `query_events` but is not
    /// capped by the streaming-row page size, so the home-screen tile shows
    /// a real count even when the user has millions of events.
    pub async fn count_events(&self, filter: EventFilter) -> Result<u64> {
        let req = pb::CountEventsRequest {
            filter: ::buffa::MessageField::some(convert::event_filter_to_pb(filter)),
            ..Default::default()
        };
        let resp = self
            .client
            .count_events_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(resp.into_owned().count.max(0) as u64)
    }

    // -------------------------------------------------------------------------
    // Grants
    // -------------------------------------------------------------------------

    /// `ListGrants`.
    pub async fn list_grants(&self, filter: ListGrantsFilter) -> Result<Vec<Grant>> {
        let req = pb::ListGrantsRequest {
            include_revoked: Some(filter.include_revoked),
            include_expired: Some(filter.include_expired),
            grantee_kind: filter.grantee_kind,
            page: filter
                .limit
                .filter(|&l| l > 0)
                .map(|l| {
                    ::buffa::MessageField::some(pb::PageRequest {
                        limit: l as i32,
                        ..Default::default()
                    })
                })
                .unwrap_or_default(),
            ..Default::default()
        };
        let resp = self
            .client
            .list_grants_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(resp
            .into_owned()
            .grants
            .into_iter()
            .map(convert::grant_from_pb)
            .collect())
    }

    /// `CreateGrant`.
    pub async fn create_grant(&self, input: CreateGrantInput) -> Result<GrantToken> {
        let req = convert::create_grant_to_pb(input);
        let resp = self
            .client
            .create_grant_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        let owned = resp.into_owned();
        let grant_ulid = owned
            .grant
            .into_option()
            .and_then(|g| g.ulid.into_option())
            .map(|u| u.bytes)
            .unwrap_or_default();
        Ok(GrantToken {
            grant_ulid,
            token: owned.token,
            share_url: owned.share_url,
        })
    }

    // -------------------------------------------------------------------------
    // Pending events
    // -------------------------------------------------------------------------

    /// `ListPending` — the user's pending-event queue (`pending` status).
    pub async fn list_pending(&self) -> Result<Vec<PendingEvent>> {
        let req = pb::ListPendingRequest {
            status: Some("pending".to_string()),
            ..Default::default()
        };
        let resp = self
            .client
            .list_pending_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(resp
            .into_owned()
            .pending
            .into_iter()
            .map(convert::pending_from_pb)
            .collect())
    }

    // -------------------------------------------------------------------------
    // Cases
    // -------------------------------------------------------------------------

    /// `ListCases`. `include_closed = None` lists open + closed.
    pub async fn list_cases(&self, include_closed: bool) -> Result<Vec<Case>> {
        let req = pb::ListCasesRequest {
            include_closed: Some(include_closed),
            ..Default::default()
        };
        let resp = self
            .client
            .list_cases_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(resp
            .into_owned()
            .cases
            .into_iter()
            .map(convert::case_from_pb)
            .collect())
    }

    /// `GetCase` by raw 16-byte ULID.
    pub async fn get_case(&self, case_ulid: Vec<u8>) -> Result<Case> {
        let req = pb::GetCaseRequest {
            case_ulid: ::buffa::MessageField::some(convert::ulid_to_pb(case_ulid)),
            ..Default::default()
        };
        let resp = self
            .client
            .get_case_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        Ok(convert::case_from_pb(resp.into_owned()))
    }

    // -------------------------------------------------------------------------
    // Audit
    // -------------------------------------------------------------------------

    /// `AuditQuery` (server-streaming) — collected into a `Vec`.
    pub async fn audit_query(&self, filter: AuditFilter) -> Result<Vec<AuditEntry>> {
        let req = pb::AuditQueryRequest {
            from_ms: filter.from_ms,
            to_ms: filter.to_ms,
            actor_type: filter.actor_type,
            action: filter.action,
            result: filter.result,
            tail: false,
            ..Default::default()
        };
        let mut stream = self
            .client
            .audit_query_with_options(req, self.auth_options())
            .await
            .map_err(map_connect_error)?;
        let mut out = Vec::new();
        while let Some(view) = stream.message().await.map_err(map_connect_error)? {
            out.push(convert::audit_entry_from_pb(view.to_owned_message()));
            if let Some(limit) = filter.limit {
                if out.len() as i64 >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    // -------------------------------------------------------------------------
    // Export
    // -------------------------------------------------------------------------

    /// `Export` (server-streaming) — drains the export stream and returns the
    /// number of frames received. Phase 1 surfaces a count rather than the
    /// raw frames: the Android binding's local `export_all` returns a CBOR
    /// buffer assembled in-core, and the wire `ExportChunk` framing is a
    /// separate (de)serialisation concern deferred to a later phase. The RPC
    /// is wired and reachable here so the count proves the stream works.
    pub async fn export(&self) -> Result<u64> {
        let mut stream = self
            .client
            .export_with_options(pb::ExportRequest::default(), self.auth_options())
            .await
            .map_err(map_connect_error)?;
        let mut frames = 0u64;
        while stream.message().await.map_err(map_connect_error)?.is_some() {
            frames += 1;
        }
        Ok(frames)
    }
}

/// Default rustls client config for `https://` endpoints: the Mozilla root
/// store, ring crypto provider, no client auth.
///
/// Roots come from the bundled `webpki-roots` Mozilla set first — this is the
/// only source that works on Android, where the OS trust store is *not* at the
/// Linux file paths [`webpki_root_certs`] scans (so that path returns nothing
/// and TLS to a public Caddy cert would otherwise fail with "unable to
/// connect"). The OS-store scan is then layered on additively, so a
/// server/desktop still trusts any private/enterprise roots it has installed.
fn default_tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert in webpki_root_certs() {
        // `add` parses the DER trust anchor; skip any cert the platform
        // bundle ships that rustls can't parse rather than failing the build.
        let _ = roots.add(cert);
    }
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// The platform / vendored trust anchors.
///
/// hyper-rustls' own `with_native_roots` / `with_webpki_roots` are gated
/// behind features we don't pull; `tokio-rustls` re-exports nothing here.
/// Rather than add a `webpki-roots` dependency, we read the OS trust store
/// when present. On a server / desktop that is the system bundle; on Android
/// the app ships its own pinned roots through the binding. For Phase 1 the
/// integration test uses plaintext `http://`, so this path is exercised only
/// by real `https://` deployments — which run behind Caddy with a publicly
/// trusted cert, resolvable from the OS store.
fn webpki_root_certs() -> Vec<rustls::pki_types::CertificateDer<'static>> {
    rustls_native_certs_load()
}

/// Load OS trust anchors, tolerating partial failure (some platforms return
/// a few unparseable certs).
fn rustls_native_certs_load() -> Vec<rustls::pki_types::CertificateDer<'static>> {
    // Best-effort: walk the common system bundle locations. Kept dependency-
    // free deliberately — the Android binding pins its own roots and the
    // server deployments are fronted by Caddy with publicly trusted certs.
    const BUNDLES: &[&str] = &[
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/cert.pem",
    ];
    for path in BUNDLES {
        if let Ok(pem) = std::fs::read(path) {
            let mut out = Vec::new();
            let mut rest: &[u8] = &pem;
            while let Some((kind, der, tail)) = next_pem_block(rest) {
                rest = tail;
                if kind == "CERTIFICATE" {
                    out.push(rustls::pki_types::CertificateDer::from(der));
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }
    Vec::new()
}

/// Minimal PEM block scanner — extracts `-----BEGIN X-----` … `-----END X-----`
/// base64 bodies. Avoids pulling `rustls-pemfile` just for trust-anchor
/// loading.
fn next_pem_block(input: &[u8]) -> Option<(String, Vec<u8>, &[u8])> {
    let text = std::str::from_utf8(input).ok()?;
    let begin = text.find("-----BEGIN ")?;
    let after_begin = &text[begin + 11..];
    let dash = after_begin.find("-----")?;
    let kind = after_begin[..dash].trim().to_string();
    let body_start = begin + 11 + dash + 5;
    let end_marker = format!("-----END {kind}-----");
    let end_rel = text[body_start..].find(&end_marker)?;
    let b64: String = text[body_start..body_start + end_rel]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let der = base64_decode(&b64)?;
    let consumed = body_start + end_rel + end_marker.len();
    Some((kind, der, &input[consumed.min(input.len())..]))
}

/// Standard base64 decode (no external dep) for PEM bodies.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc = 0u32;
    let mut nbits = 0u32;
    for b in bytes {
        let v = val(b)?;
        acc = (acc << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Some(out)
}
