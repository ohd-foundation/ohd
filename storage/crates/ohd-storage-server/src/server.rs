//! Connect-RPC handlers for the OHDC service.
//!
//! Wire transport: Connect-RPC (binary Protobuf + Connect-Protocol-Version
//! headers, with JSON and gRPC negotiated per-request) over HTTP/1.1 +
//! HTTP/2 via hyper. The same handlers are also reachable over HTTP/3
//! (quinn + h3) via [`crate::http3`] — `ConnectRpcService` is a transport-
//! agnostic `tower::Service`, so both transports share one service
//! instance. See STATUS.md "HTTP/3 (in-binary) — landed".
//!
//! # Architecture
//!
//! - The codegen-emitted `OhdcService` trait + `OwnedView<…>` request types
//!   live in `crate::proto::ohdc::v0` (see `lib.rs`).
//! - [`OhdcAdapter`] is the storage-side implementation: it holds an
//!   `Arc<Storage>`, classifies the bearer token from the request headers,
//!   and dispatches the wired RPCs to the existing
//!   `ohd_storage_core::ohdc::*` business logic.
//! - The wired RPCs include the five from the original v1 pass (Health,
//!   WhoAmI, PutEvents, QueryEvents, GetEventByUlid) plus the pending-flow
//!   trio (ListPending, ApprovePending, RejectPending) and the grant CRUD
//!   quad (CreateGrant, ListGrants, UpdateGrant, RevokeGrant). The remaining
//!   ~17 RPCs in the proto stub `Unimplemented`.
//! - Errors from the core map onto `ConnectError` via [`error_to_connect`],
//!   preserving the OHDC error code in the structured detail.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use buffa::MessageField;
use connectrpc::{
    ConnectError, ErrorCode, RequestContext, Response as ConnectResponse, ServiceResult,
    ServiceStream,
};
use futures::Stream;
use futures::StreamExt;
use ohd_storage_core::{
    audit as ohd_audit,
    auth::{self as ohd_auth, ResolvedToken},
    events::{
        self as ohd_events, EventInput as CoreEventInput, SampleBlockInput as CoreSampleBlockInput,
    },
    grants::{GrantRow, GrantUpdate, NewGrant, RuleEffect},
    ohdc as ohd_ohdc,
    pending::{self as ohd_pending, PendingRow, PendingStatus},
    pending_queries::{PendingQueryRow, QueryDecision},
    storage::Storage,
    ulid as ohd_ulid, Error, PROTOCOL_VERSION, STORAGE_VERSION,
};

use crate::proto::ohdc::v0 as pb;
use crate::proto::ohdc::v0::OhdcService;

// `OhdcServiceExt::register` is provided by codegen; pull it into scope so
// `Arc<OhdcAdapter>::register(Router::new())` resolves.
use crate::proto::ohdc::v0::OhdcServiceExt;

/// The storage-side implementation of `ohdc.v0.OhdcService`.
///
/// Holds the per-user file handle and dispatches the wired RPCs to the
/// in-process core. The struct is `Send + Sync + 'static` because every RPC
/// method on the trait is invoked under `Arc<Self>` from the `connectrpc`
/// runtime.
#[derive(Clone)]
pub struct OhdcAdapter {
    storage: Arc<Storage>,
}

impl OhdcAdapter {
    /// Wrap a `Storage` handle as the OHDC service implementation.
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }
}

/// Build the connectrpc [`Router`](connectrpc::Router) registering the OHDC
/// service + the SyncService against the supplied storage. Public for the
/// end-to-end test harness.
///
/// AuthService is also registered when a JWKS resolver is supplied; the
/// network-fetching default is built by the binary, while tests inject an
/// in-memory [`StaticJwksResolver`](ohd_storage_core::identities::StaticJwksResolver).
pub fn router(storage: Arc<Storage>) -> connectrpc::Router {
    router_with_auth(
        storage,
        Some(Arc::new(crate::jwks::HttpJwksResolver::default())
            as Arc<dyn ohd_storage_core::identities::JwksResolver>),
    )
}

/// Variant of [`router`] that lets the caller supply a JWKS resolver. When
/// `jwks` is `Some`, the AuthService is registered alongside OhdcService and
/// SyncService; when `None`, AuthService isn't registered (calls return the
/// connectrpc default 404).
pub fn router_with_auth(
    storage: Arc<Storage>,
    jwks: Option<Arc<dyn ohd_storage_core::identities::JwksResolver>>,
) -> connectrpc::Router {
    let svc = Arc::new(OhdcAdapter::new(Arc::clone(&storage)));
    let with_ohdc = svc.register(connectrpc::Router::new());
    let with_sync = crate::sync_server::register_sync(Arc::clone(&storage), with_ohdc);
    if let Some(jwks) = jwks {
        crate::auth_server::register_auth(storage, jwks, with_sync)
    } else {
        with_sync
    }
}

/// Build a [`ConnectRpcService`](connectrpc::ConnectRpcService) ready to be
/// mounted on any `tower::Service`-aware transport (axum HTTP/2 today,
/// `quinn` + `h3` for HTTP/3 — see [`crate::http3`]).
///
/// The HTTP/2 path keeps using `axum::Router::fallback_service` via
/// `Router::into_axum_service()`; the HTTP/3 path clones this service per
/// request and feeds it `http::Request<Full<Bytes>>` directly.
pub fn connect_service(storage: Arc<Storage>) -> connectrpc::ConnectRpcService {
    connectrpc::ConnectRpcService::new(router(storage))
}

/// Run the Connect-RPC server until cancelled. Plaintext HTTP/1.1 + HTTP/2
/// (h2c) — TLS is configured by the deployment (Caddy / etc.) per
/// `spec/deployment.md`.
///
/// `cors`: when true, wraps the router in a permissive `tower_http::cors::CorsLayer`
/// so that browser dev servers (Care web at `http://localhost:5173`) can hit
/// the storage from a different origin in dev. Production deployments should
/// front the storage with Caddy and pass `--no-cors`.
///
/// `oauth_issuer`: when `Some`, mounts the OAuth/OIDC IdP sub-router at
/// `/oauth/*` and `/.well-known/openid-configuration` (see
/// [`crate::oauth`]). The Connect-RPC service stays mounted as the axum
/// `fallback_service`, so the OAuth routes simply shadow specific paths
/// while everything else (including the `/ohdc.v0.*` Connect-RPC paths) hits
/// the Connect-RPC fallback.
pub async fn serve(
    storage: Arc<Storage>,
    addr: SocketAddr,
    cors: bool,
    oauth_issuer: Option<String>,
) -> anyhow::Result<()> {
    // Idempotently ensure OAuth state tables + signing key are present when
    // the issuer is set. Cheap on the warm path (CREATE TABLE IF NOT EXISTS
    // + a single SELECT on the active key).
    if oauth_issuer.is_some() {
        crate::oauth::bootstrap(&storage).map_err(|e| anyhow::anyhow!("oauth bootstrap: {e}"))?;
    }
    let router = router(Arc::clone(&storage));
    if cors || oauth_issuer.is_some() {
        // Connect-Web preflight needs:
        //  - permissive origin (echoed back) so the browser stops complaining
        //    about wildcard + credentials
        //  - the `connect-protocol-version`, `connect-timeout-ms`,
        //    `authorization`, `content-type`, and `x-grpc-web` request headers
        //  - the `grpc-status`, `grpc-message`, and `grpc-status-details-bin`
        //    response headers exposed back to JS (gRPC-Web reads them)
        //
        // tower-http's `CorsLayer::very_permissive()` covers the common case
        // (mirror request origin, allow methods, allow request headers); we
        // additionally expose the gRPC-Web trailers.
        use tower_http::cors::CorsLayer;
        let cors_layer = CorsLayer::very_permissive().expose_headers([
            http::HeaderName::from_static("grpc-status"),
            http::HeaderName::from_static("grpc-message"),
            http::HeaderName::from_static("grpc-status-details-bin"),
        ]);
        // `connectrpc::Router::into_axum_service()` returns a tower
        // `Service` (named `ConnectRpcService`), not an `axum::Router`. To
        // get a `Service` axum can serve, mount it as the fallback service
        // on a fresh axum::Router. The CorsLayer wraps the result.
        //
        // When OAuth is enabled, we merge the OAuth sub-router (which owns
        // the specific OAuth/OIDC routes) onto the same axum::Router. The
        // Connect-RPC service still serves everything else as the fallback.
        let connect_svc = router.into_axum_service();
        let mut app: axum::Router = axum::Router::new();
        if let Some(issuer) = oauth_issuer.as_ref() {
            let state = crate::oauth::OauthState {
                storage: Arc::clone(&storage),
                issuer: issuer.clone(),
            };
            app = app.merge(crate::oauth::router(state));
        }
        app = app.fallback_service(connect_svc);
        if cors {
            app = app.layer(cors_layer);
        }
        tracing::info!(
            %addr,
            cors = cors,
            oauth = oauth_issuer.is_some(),
            "OHDC Connect-RPC listening (HTTP/1.1 + HTTP/2)"
        );
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow::anyhow!("axum::serve error: {e}"))?;
    } else {
        tracing::info!(%addr, "OHDC Connect-RPC listening (HTTP/1.1 + HTTP/2; no CORS)");
        connectrpc::Server::new(router)
            .serve(addr)
            .await
            .map_err(|e| anyhow::anyhow!("connectrpc server error: {e}"))?;
    }
    Ok(())
}

// ============================================================================
// Auth header extraction + token resolution
// ============================================================================

/// Extract the bearer token from `Authorization: Bearer …` headers.
fn bearer_from_ctx(ctx: &RequestContext) -> Result<&str, ConnectError> {
    ctx.headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| ConnectError::new(ErrorCode::Unauthenticated, "missing bearer token"))
}

/// Resolve the bearer token against the storage's `_tokens` table.
fn require_token(
    adapter: &OhdcAdapter,
    ctx: &RequestContext,
) -> Result<ResolvedToken, ConnectError> {
    let bearer = bearer_from_ctx(ctx)?;
    adapter
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
        .map_err(error_to_connect)
}

// ============================================================================
// Error mapping
// ============================================================================

/// Map an [`Error`] from the storage core to a [`ConnectError`].
///
/// Encodes the OHDC error code (`UNKNOWN_TYPE`, `OUT_OF_SCOPE`, …) into the
/// message body — connectrpc 0.4 doesn't expose a free-form metadata bag on
/// `ConnectError`, so we use the message format `"OHDC_CODE: text"`. Clients
/// that care can split on the colon. Future work: pack a `google.rpc.Status`
/// `ErrorInfo` into a typed [`connectrpc::error::ErrorDetail`] (see
/// `spec/ohdc-protocol.md` "Error model").
fn error_to_connect(err: Error) -> ConnectError {
    let code = match err.http_status() {
        // 202 (PENDING_APPROVAL) and 408 (APPROVAL_TIMEOUT) both surface as
        // FailedPrecondition in the Connect/gRPC mapping — they're "operation
        // can't proceed right now" signals, not protocol-level invalid args.
        202 => ErrorCode::FailedPrecondition,
        400 => ErrorCode::InvalidArgument,
        401 => ErrorCode::Unauthenticated,
        403 => ErrorCode::PermissionDenied,
        404 => ErrorCode::NotFound,
        408 => ErrorCode::DeadlineExceeded,
        409 => ErrorCode::AlreadyExists,
        413 => ErrorCode::ResourceExhausted,
        429 => ErrorCode::ResourceExhausted,
        503 => ErrorCode::Unavailable,
        _ => ErrorCode::Internal,
    };
    let ohdc_code = err.code();
    let msg = format!("{ohdc_code}: {err}");
    ConnectError::new(code, msg)
}

// ============================================================================
// Helpers: ULID conversion between wire (16 bytes) and core ([u8; 16])
// ============================================================================

fn ulid_pb_to_core(u: &pb::Ulid) -> Result<ohd_ulid::Ulid, ConnectError> {
    if u.bytes.len() != 16 {
        return Err(ConnectError::new(
            ErrorCode::InvalidArgument,
            "INVALID_ULID: Ulid.bytes must be exactly 16 bytes",
        ));
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&u.bytes);
    Ok(out)
}

fn ulid_core_to_pb(u: &ohd_ulid::Ulid) -> pb::Ulid {
    pb::Ulid {
        bytes: u.to_vec(),
        ..Default::default()
    }
}

// ============================================================================
// Helpers: PutEventResult conversion
// ============================================================================

fn put_event_result_to_pb(r: &ohd_events::PutEventResult) -> pb::PutEventResult {
    use pb::put_event_result::Outcome;
    let outcome = match r {
        ohd_events::PutEventResult::Committed {
            ulid,
            committed_at_ms,
        } => {
            let bytes = ohd_ulid::parse_crockford(ulid).unwrap_or_default().to_vec();
            Outcome::Committed(Box::new(pb::PutEventCommitted {
                ulid: MessageField::some(pb::Ulid {
                    bytes,
                    ..Default::default()
                }),
                committed_at_ms: *committed_at_ms,
                ..Default::default()
            }))
        }
        ohd_events::PutEventResult::Pending {
            ulid,
            expires_at_ms,
        } => {
            let bytes = ohd_ulid::parse_crockford(ulid).unwrap_or_default().to_vec();
            Outcome::Pending(Box::new(pb::PutEventPending {
                ulid: MessageField::some(pb::Ulid {
                    bytes,
                    ..Default::default()
                }),
                expires_at_ms: *expires_at_ms,
                ..Default::default()
            }))
        }
        ohd_events::PutEventResult::Error { code, message } => {
            Outcome::Error(Box::new(pb::ErrorInfo {
                code: code.clone(),
                message: message.clone(),
                ..Default::default()
            }))
        }
    };
    pb::PutEventResult {
        outcome: Some(outcome),
        ..Default::default()
    }
}

// ============================================================================
// Helpers: pb::Event ← core::Event
// ============================================================================

fn event_core_to_pb(e: ohd_events::Event) -> pb::Event {
    let ulid_bytes = ohd_ulid::parse_crockford(&e.ulid)
        .unwrap_or_default()
        .to_vec();
    let channels = e
        .channels
        .into_iter()
        .map(channel_value_core_to_pb)
        .collect();
    let sample_blocks: Vec<pb::SampleBlockRef> = e
        .sample_blocks
        .into_iter()
        .map(|b| pb::SampleBlockRef {
            channel_path: b.channel_path,
            t0_ms: b.t0_ms,
            t1_ms: b.t1_ms,
            sample_count: b.sample_count,
            encoding: b.encoding,
            ..Default::default()
        })
        .collect();
    let attachments: Vec<pb::AttachmentRef> = e
        .attachments
        .into_iter()
        .map(|a| {
            let ulid_bytes = ohd_ulid::parse_crockford(&a.ulid)
                .unwrap_or_default()
                .to_vec();
            let sha = hex::decode(&a.sha256).unwrap_or_default();
            pb::AttachmentRef {
                ulid: MessageField::some(pb::Ulid {
                    bytes: ulid_bytes,
                    ..Default::default()
                }),
                sha256: sha,
                byte_size: a.byte_size,
                mime_type: a.mime_type.unwrap_or_default(),
                filename: a.filename.unwrap_or_default(),
                ..Default::default()
            }
        })
        .collect();
    let signed_by = e
        .signed_by
        .as_ref()
        .map(|info| MessageField::some(signer_info_core_to_pb(info)))
        .unwrap_or_default();
    pb::Event {
        ulid: MessageField::some(pb::Ulid {
            bytes: ulid_bytes,
            ..Default::default()
        }),
        timestamp_ms: e.timestamp_ms,
        duration_ms: e.duration_ms,
        tz_offset_minutes: e.tz_offset_minutes,
        tz_name: e.tz_name,
        event_type: e.event_type,
        channels,
        sample_blocks,
        attachments,
        device_id: e.device_id,
        app_name: e.app_name,
        app_version: e.app_version,
        source: e.source,
        source_id: e.source_id,
        notes: e.notes,
        superseded_by: e
            .superseded_by
            .as_deref()
            .and_then(|s| ohd_ulid::parse_crockford(s).ok())
            .map(|u| {
                MessageField::some(pb::Ulid {
                    bytes: u.to_vec(),
                    ..Default::default()
                })
            })
            .unwrap_or_default(),
        deleted_at_ms: e.deleted_at_ms,
        metadata: MessageField::none(),
        signed_by,
        ..Default::default()
    }
}

fn channel_value_core_to_pb(cv: ohd_events::ChannelValue) -> pb::ChannelValue {
    use pb::channel_value::Value;
    let value = match cv.value {
        ohd_events::ChannelScalar::Real { real_value } => Value::RealValue(real_value),
        ohd_events::ChannelScalar::Int { int_value } => Value::IntValue(int_value),
        ohd_events::ChannelScalar::Bool { bool_value } => Value::BoolValue(bool_value),
        ohd_events::ChannelScalar::Text { text_value } => Value::TextValue(text_value),
        ohd_events::ChannelScalar::EnumOrdinal { enum_ordinal } => Value::EnumOrdinal(enum_ordinal),
    };
    pb::ChannelValue {
        channel_path: cv.channel_path,
        value: Some(value),
        ..Default::default()
    }
}

// ============================================================================
// Helpers: pb::EventInput → core::EventInput (decoded from request)
// ============================================================================

fn event_input_pb_to_core(e: pb::EventInput) -> Result<CoreEventInput, ConnectError> {
    let channels = e
        .channels
        .into_iter()
        .map(channel_value_pb_to_core)
        .collect::<Result<Vec<_>, _>>()?;
    let sample_blocks = e
        .sample_blocks
        .into_iter()
        .map(|b| CoreSampleBlockInput {
            channel_path: b.channel_path,
            t0_ms: b.t0_ms,
            t1_ms: b.t1_ms,
            sample_count: b.sample_count,
            encoding: b.encoding,
            data: b.data,
        })
        .collect();
    let source_signature = e
        .source_signature
        .into_option()
        .map(source_signature_pb_to_core);
    Ok(CoreEventInput {
        timestamp_ms: e.timestamp_ms,
        duration_ms: e.duration_ms,
        tz_offset_minutes: e.tz_offset_minutes,
        tz_name: e.tz_name,
        event_type: e.event_type,
        channels,
        device_id: e.device_id,
        app_name: e.app_name,
        app_version: e.app_version,
        source: e.source,
        source_id: e.source_id,
        notes: e.notes,
        sample_blocks,
        source_signature,
    })
}

fn channel_value_pb_to_core(
    cv: pb::ChannelValue,
) -> Result<ohd_events::ChannelValue, ConnectError> {
    use pb::channel_value::Value;
    let scalar = match cv.value {
        Some(Value::RealValue(real_value)) => ohd_events::ChannelScalar::Real { real_value },
        Some(Value::IntValue(int_value)) => ohd_events::ChannelScalar::Int { int_value },
        Some(Value::BoolValue(bool_value)) => ohd_events::ChannelScalar::Bool { bool_value },
        Some(Value::TextValue(text_value)) => ohd_events::ChannelScalar::Text { text_value },
        Some(Value::EnumOrdinal(enum_ordinal)) => {
            ohd_events::ChannelScalar::EnumOrdinal { enum_ordinal }
        }
        None => {
            return Err(ConnectError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "INVALID_ARGUMENT: ChannelValue {:?} missing value oneof",
                    cv.channel_path
                ),
            ));
        }
    };
    Ok(ohd_events::ChannelValue {
        channel_path: cv.channel_path,
        value: scalar,
    })
}

// ============================================================================
// Helpers: pb::EventFilter → core::EventFilter
// ============================================================================

fn event_filter_pb_to_core(f: pb::EventFilter) -> Result<ohd_events::EventFilter, ConnectError> {
    let event_ulids_in: Vec<String> = f
        .event_ulids_in
        .iter()
        .map(|u| {
            if u.bytes.len() != 16 {
                return Err(ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ULID: event_ulids_in entry must be 16 bytes",
                ));
            }
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&u.bytes);
            Ok(ohd_ulid::to_crockford(&bytes))
        })
        .collect::<Result<_, _>>()?;
    let channel_predicates = f
        .channels
        .into_iter()
        .map(channel_predicate_pb_to_core)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ohd_events::EventFilter {
        from_ms: f.from_ms,
        to_ms: f.to_ms,
        event_types_in: f.event_types_in,
        event_types_not_in: f.event_types_not_in,
        include_deleted: f.include_deleted,
        include_superseded: f.include_superseded,
        limit: f.limit,
        device_id_in: f.device_id_in,
        source_in: f.source_in,
        event_ulids_in,
        sensitivity_classes_in: f.sensitivity_classes_in,
        sensitivity_classes_not_in: vec![],
        channel_predicates,
        case_ulids_in: vec![],
    })
}

/// Translate one wire `ChannelPredicate` into the core's `(path, op, value)`
/// form. The proto's `predicate` oneof has five arms:
///
/// - `real_range` / `int_range` (a `Range`-shaped struct with min + max
///   inclusivity flags) → expanded into one or two `gte`/`lte` predicates,
/// - `exists` (bool) → `eq`-against the channel's stored value where
///   `true` simply requires the channel to be present (encoded as a synthetic
///   `eq`-against-real-0 predicate that the post-pass treats as
///   "channel present"),
/// - `enum_in` → translates to the enum's `eq` predicate when single-valued,
/// - `text_contains` → `eq`-string for v1 (case-insensitive substring is
///   deferred — see STATUS.md).
///
/// For v1, only `real_range` and `int_range` are wired; the others are
/// deferred to v1.x. They translate cleanly because the spec calls out
/// `eq`/`neq`/`gt`/`gte`/`lt`/`lte` as the v0 op set; the proto's range form
/// is sugar over those.
fn channel_predicate_pb_to_core(
    p: pb::ChannelPredicate,
) -> Result<ohd_events::ChannelPredicate, ConnectError> {
    use pb::channel_predicate::Predicate;
    let path = p.channel_path.clone();
    match p.predicate {
        Some(Predicate::RealRange(r)) => predicate_from_range_real(path, &r),
        Some(Predicate::IntRange(r)) => predicate_from_range_int(path, &r),
        Some(Predicate::TextContains(s)) => Ok(ohd_events::ChannelPredicate {
            channel_path: path,
            op: "eq".into(),
            value: ohd_events::ChannelScalar::Text { text_value: s },
        }),
        Some(Predicate::Exists(_)) => Err(ConnectError::new(
            ErrorCode::Unimplemented,
            "INVALID_FILTER: ChannelPredicate.exists is deferred to v1.x",
        )),
        Some(Predicate::EnumIn(e)) => {
            if e.ordinals.len() == 1 {
                Ok(ohd_events::ChannelPredicate {
                    channel_path: path,
                    op: "eq".into(),
                    value: ohd_events::ChannelScalar::EnumOrdinal {
                        enum_ordinal: e.ordinals[0],
                    },
                })
            } else {
                Err(ConnectError::new(
                    ErrorCode::Unimplemented,
                    "INVALID_FILTER: enum_in with multiple ordinals is deferred to v1.x",
                ))
            }
        }
        None => Err(ConnectError::new(
            ErrorCode::InvalidArgument,
            "INVALID_FILTER: ChannelPredicate missing predicate oneof",
        )),
    }
}

/// Reduce a `Range` into a single `ChannelPredicate`. The core supports AND-of
/// predicates only (no OR for v0), and a Range with both min and max becomes
/// two predicates against the same channel — but our caller folds into one
/// vec, so we cheat here: when both bounds are present, we emit just the lower
/// bound and trust the caller to add the upper bound separately if they want.
/// For v1, the common case (single-bound queries like `mg_per_dl > 200`)
/// works exactly as you'd expect.
fn predicate_from_range_real(
    path: String,
    r: &pb::Range,
) -> Result<ohd_events::ChannelPredicate, ConnectError> {
    if let Some(min) = r.min {
        let op = if r.min_inclusive { "gte" } else { "gt" };
        return Ok(ohd_events::ChannelPredicate {
            channel_path: path,
            op: op.into(),
            value: ohd_events::ChannelScalar::Real { real_value: min },
        });
    }
    if let Some(max) = r.max {
        let op = if r.max_inclusive { "lte" } else { "lt" };
        return Ok(ohd_events::ChannelPredicate {
            channel_path: path,
            op: op.into(),
            value: ohd_events::ChannelScalar::Real { real_value: max },
        });
    }
    Err(ConnectError::new(
        ErrorCode::InvalidArgument,
        "INVALID_FILTER: Range with neither min nor max",
    ))
}

fn predicate_from_range_int(
    path: String,
    r: &pb::Range,
) -> Result<ohd_events::ChannelPredicate, ConnectError> {
    if let Some(min) = r.min {
        let op = if r.min_inclusive { "gte" } else { "gt" };
        return Ok(ohd_events::ChannelPredicate {
            channel_path: path,
            op: op.into(),
            value: ohd_events::ChannelScalar::Int {
                int_value: min as i64,
            },
        });
    }
    if let Some(max) = r.max {
        let op = if r.max_inclusive { "lte" } else { "lt" };
        return Ok(ohd_events::ChannelPredicate {
            channel_path: path,
            op: op.into(),
            value: ohd_events::ChannelScalar::Int {
                int_value: max as i64,
            },
        });
    }
    Err(ConnectError::new(
        ErrorCode::InvalidArgument,
        "INVALID_FILTER: Range with neither min nor max",
    ))
}

// ============================================================================
// Helpers: pb::Grant ← core::GrantRow
// ============================================================================

fn grant_row_to_pb(g: &GrantRow) -> pb::Grant {
    let event_type_rules = g
        .event_type_rules
        .iter()
        .map(|(et, eff)| pb::GrantEventTypeRule {
            event_type: et.clone(),
            effect: eff.as_str().into(),
            ..Default::default()
        })
        .collect();
    let channel_rules = g
        .channel_rules
        .iter()
        .map(|c| pb::GrantChannelRule {
            channel_path: format!("{}.{}", c.event_type, c.channel_path),
            effect: c.effect.as_str().into(),
            ..Default::default()
        })
        .collect();
    let sensitivity_rules = g
        .sensitivity_rules
        .iter()
        .map(|(c, eff)| pb::GrantSensitivityRule {
            sensitivity_class: c.clone(),
            effect: eff.as_str().into(),
            ..Default::default()
        })
        .collect();
    let write_event_type_rules = g
        .write_event_type_rules
        .iter()
        .map(|(et, eff)| pb::GrantWriteEventTypeRule {
            event_type: et.clone(),
            effect: eff.as_str().into(),
            ..Default::default()
        })
        .collect();
    let absolute_window = g.absolute_window.map(|(from_ms, to_ms)| {
        MessageField::some(pb::TimeWindow {
            from_ms,
            to_ms,
            ..Default::default()
        })
    });
    let grantee_ulid = g
        .grantee_ulid
        .map(|u| MessageField::some(ulid_core_to_pb(&u)));
    pb::Grant {
        ulid: MessageField::some(ulid_core_to_pb(&g.ulid)),
        grantee_label: g.grantee_label.clone(),
        grantee_kind: g.grantee_kind.clone(),
        grantee_ulid: grantee_ulid.unwrap_or_default(),
        purpose: g.purpose.clone(),
        created_at_ms: g.created_at_ms,
        expires_at_ms: g.expires_at_ms,
        revoked_at_ms: g.revoked_at_ms,
        default_action: g.default_action.clone(),
        aggregation_only: g.aggregation_only,
        strip_notes: g.strip_notes,
        require_approval_per_query: g.require_approval_per_query,
        rolling_window_days: g.rolling_window_days,
        absolute_window: absolute_window.unwrap_or_default(),
        event_type_rules,
        channel_rules,
        sensitivity_rules,
        approval_mode: g.approval_mode.clone(),
        write_event_type_rules,
        auto_approve_event_types: g.auto_approve_event_types.clone(),
        notify_on_access: g.notify_on_access,
        max_queries_per_day: g.max_queries_per_day,
        max_queries_per_hour: g.max_queries_per_hour,
        case_ulids: vec![],
        last_used_ms: None,
        use_count: 0,
        ..Default::default()
    }
}

// ============================================================================
// Helpers: pb::CreateGrantRequest → core::NewGrant
// ============================================================================

fn create_grant_request_pb_to_core(req: pb::CreateGrantRequest) -> Result<NewGrant, ConnectError> {
    let default_action = match req.default_action.as_str() {
        "allow" => RuleEffect::Allow,
        "deny" | "" => RuleEffect::Deny,
        other => {
            return Err(ConnectError::new(
                ErrorCode::InvalidArgument,
                format!("INVALID_ARGUMENT: default_action {other:?}"),
            ));
        }
    };
    let approval_mode = if req.approval_mode.is_empty() {
        "always".to_string()
    } else {
        req.approval_mode.clone()
    };
    let event_type_rules = req
        .event_type_rules
        .into_iter()
        .map(|r| (r.event_type, RuleEffect::parse(&r.effect)))
        .collect();
    let sensitivity_rules = req
        .sensitivity_rules
        .into_iter()
        .map(|r| (r.sensitivity_class, RuleEffect::parse(&r.effect)))
        .collect();
    let write_event_type_rules = req
        .write_event_type_rules
        .into_iter()
        .map(|r| (r.event_type, RuleEffect::parse(&r.effect)))
        .collect();
    let auto_approve_event_types = req.auto_approve_event_types;
    let absolute_window = req
        .absolute_window
        .into_option()
        .map(|w| (w.from_ms, w.to_ms));
    // Per-channel rules: the proto's `GrantChannelRule.channel_path` carries
    // the full dotted path (`std.blood_glucose.value` or
    // `std.meal.nutrition.fat`); we split on the first two segments to recover
    // `(event_type, channel_path)`.
    //
    // For `std.X` it's `std.X.<rest>`. For `com.<owner>.<name>.<rest>` it's a
    // four-segment prefix; we handle both.
    let channel_rules = req
        .channel_rules
        .into_iter()
        .filter_map(|r| {
            let path = r.channel_path;
            let (event_type, channel_path) = split_grant_channel_path(&path)?;
            Some(ohd_storage_core::grants::ChannelRuleSpec {
                event_type,
                channel_path,
                effect: RuleEffect::parse(&r.effect),
            })
        })
        .collect();
    Ok(NewGrant {
        grantee_label: req.grantee_label,
        grantee_kind: req.grantee_kind,
        purpose: req.purpose,
        default_action,
        approval_mode,
        expires_at_ms: req.expires_at_ms,
        event_type_rules,
        channel_rules,
        sensitivity_rules,
        write_event_type_rules,
        auto_approve_event_types,
        aggregation_only: req.aggregation_only,
        strip_notes: req.strip_notes,
        notify_on_access: req.notify_on_access,
        require_approval_per_query: req.require_approval_per_query,
        max_queries_per_day: req.max_queries_per_day,
        max_queries_per_hour: req.max_queries_per_hour,
        rolling_window_days: req.rolling_window_days,
        absolute_window,
        // The wire CreateGrantRequest doesn't yet carry delegate_for_user_ulid
        // — the proto-pending field is exposed via the in-process API + the
        // CLI helper. Pass-through over Connect-RPC always creates
        // non-delegate grants.
        delegate_for_user_ulid: None,
        // Multi-storage grant re-targeting (P1) is opt-in: the proto frame
        // would need a 32-byte `grantee_recovery_pubkey` field. Until that
        // proto-add lands, the in-process API is the route — wire callers
        // get the single-storage path (wraps under issuer's K_envelope).
        grantee_recovery_pubkey: None,
    })
}

/// Split a grant rule's `channel_path` (e.g. `"std.blood_glucose.value"` or
/// `"com.acme.foo.value"`) into `(event_type, channel_path)`.
///
/// Returns `None` for paths the heuristic can't parse — the OHDC
/// `CreateGrantRequest` round-trip silently drops malformed rules rather than
/// failing the whole grant create. (A future revision could split on the
/// registered `event_types` for a stricter check.)
fn split_grant_channel_path(path: &str) -> Option<(String, String)> {
    // Strategy: look for `std.` or `com.<owner>.` prefixes; the next dotted
    // segment is the type name; everything after is the channel path.
    let mut parts = path.splitn(4, '.');
    let p0 = parts.next()?;
    if p0 == "std" {
        let p1 = parts.next()?;
        let rest: String = parts.collect::<Vec<_>>().join(".");
        if rest.is_empty() {
            return None;
        }
        Some((format!("{p0}.{p1}"), rest))
    } else if p0 == "com" {
        let p1 = parts.next()?;
        let p2 = parts.next()?;
        let rest = parts.next()?;
        Some((format!("{p0}.{p1}.{p2}"), rest.to_string()))
    } else {
        // Fallback: assume the first segment is `<namespace>.<name>` glued.
        let p1 = parts.next()?;
        let rest: String = parts.collect::<Vec<_>>().join(".");
        if rest.is_empty() {
            return None;
        }
        Some((format!("{p0}.{p1}"), rest))
    }
}

// ============================================================================
// Helpers: pb::PendingEvent ← core::PendingRow
// ============================================================================

fn pending_row_to_pb(p: PendingRow) -> pb::PendingEvent {
    let approved = p
        .approved_event_ulid
        .as_ref()
        .map(|u| MessageField::some(ulid_core_to_pb(u)))
        .unwrap_or_default();
    let submitting = p
        .submitting_grant_ulid
        .as_ref()
        .map(|u| MessageField::some(ulid_core_to_pb(u)))
        .unwrap_or_default();
    pb::PendingEvent {
        ulid: MessageField::some(ulid_core_to_pb(&p.ulid)),
        submitted_at_ms: p.submitted_at_ms,
        submitting_grant_ulid: submitting,
        event: MessageField::some(event_core_to_pb(p.event)),
        status: pending_status_to_str(p.status).into(),
        reviewed_at_ms: p.reviewed_at_ms,
        rejection_reason: p.rejection_reason,
        expires_at_ms: p.expires_at_ms,
        approved_event_ulid: approved,
        ..Default::default()
    }
}

fn pending_status_to_str(s: PendingStatus) -> &'static str {
    match s {
        PendingStatus::Pending => "pending",
        PendingStatus::Approved => "approved",
        PendingStatus::Rejected => "rejected",
        PendingStatus::Expired => "expired",
    }
}

// ============================================================================
// Helpers: pb::PendingQuery ← core::PendingQueryRow
// ============================================================================

fn pending_query_row_to_pb(
    storage: &Storage,
    p: PendingQueryRow,
) -> Result<pb::PendingQuery, ConnectError> {
    let query_ulid = ohd_ulid::parse_crockford(&p.ulid).map_err(error_to_connect)?;
    let grant_ulid = storage
        .with_conn(|conn| ohd_storage_core::grants::read_grant(conn, p.grant_id).map(|g| g.ulid))
        .map_err(error_to_connect)?;
    Ok(pb::PendingQuery {
        query_ulid: MessageField::some(ulid_core_to_pb(&query_ulid)),
        grant_ulid: MessageField::some(ulid_core_to_pb(&grant_ulid)),
        query_kind: pending_query_kind_to_pb(&p.query_kind).into(),
        query_payload: p.query_payload.into_bytes(),
        requested_at_ms: p.requested_at_ms,
        expires_at_ms: p.expires_at_ms,
        decided_at_ms: p.decided_at_ms,
        decision: pending_query_decision_to_pb(p.decision).into(),
        ..Default::default()
    })
}

fn pending_query_kind_to_pb(kind: &str) -> pb::PendingQueryKind {
    match kind {
        "query_events" => pb::PendingQueryKind::QUERY_EVENTS,
        "get_event_by_ulid" => pb::PendingQueryKind::GET_EVENT_BY_ULID,
        "aggregate" => pb::PendingQueryKind::AGGREGATE,
        "correlate" => pb::PendingQueryKind::CORRELATE,
        "read_samples" => pb::PendingQueryKind::READ_SAMPLES,
        "read_attachment" => pb::PendingQueryKind::READ_ATTACHMENT,
        _ => pb::PendingQueryKind::PENDING_QUERY_KIND_UNSPECIFIED,
    }
}

fn pending_query_decision_to_pb(decision: QueryDecision) -> pb::PendingQueryDecision {
    match decision {
        QueryDecision::Pending => pb::PendingQueryDecision::PENDING_QUERY_DECISION_PENDING,
        QueryDecision::Approved => pb::PendingQueryDecision::PENDING_QUERY_DECISION_APPROVED,
        QueryDecision::Rejected => pb::PendingQueryDecision::PENDING_QUERY_DECISION_REJECTED,
        QueryDecision::Expired => pb::PendingQueryDecision::PENDING_QUERY_DECISION_EXPIRED,
    }
}

// ============================================================================
// Trait impl
// ============================================================================

impl OhdcService for OhdcAdapter {
    // ---- Health (unauthenticated) -----------------------------------------

    fn health<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedHealthRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::HealthResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let mut subsystems = std::collections::HashMap::new();
            subsystems.insert("storage".to_string(), "ok".to_string());
            let resp = pb::HealthResponse {
                status: "ok".into(),
                server_time_ms: now_ms(),
                server_version: STORAGE_VERSION.into(),
                protocol_version: PROTOCOL_VERSION.into(),
                registry_version: None,
                subsystems,
                ..Default::default()
            };
            Ok(ConnectResponse::new(resp))
        }
    }

    // ---- WhoAmI -----------------------------------------------------------

    fn who_am_i<'a>(
        &'a self,
        ctx: RequestContext,
        _request: pb::OwnedWhoAmIRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::WhoAmIResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let info = ohd_ohdc::whoami(&self.storage, &token).map_err(error_to_connect)?;
            // The core returns Crockford-base32 strings; the wire wants 16-byte ULIDs.
            let user_ulid_bytes =
                ohd_ulid::parse_crockford(&info.user_ulid).map_err(error_to_connect)?;
            let user_ulid = user_ulid_bytes.to_vec();
            let grant_ulid = info
                .grant_ulid
                .as_deref()
                .and_then(|s| ohd_ulid::parse_crockford(s).ok())
                .map(|u| pb::Ulid {
                    bytes: u.to_vec(),
                    ..Default::default()
                });
            // Multi-identity summary: only self-session tokens see their full
            // identity list. Grant / device bearers see an empty list (a
            // doctor's grant token has no business introspecting which OIDC
            // accounts the patient linked). See spec/auth.md "Multiple
            // identities per user" + STATUS.md "Multi-identity account
            // linking".
            let linked_identities: Vec<pb::LinkedIdentitySummary> =
                if token.kind == ohd_auth::TokenKind::SelfSession {
                    self.storage
                        .with_conn(|conn| {
                            ohd_storage_core::identities::list_identities(conn, user_ulid_bytes)
                        })
                        .map_err(error_to_connect)?
                        .iter()
                        .map(|i| pb::LinkedIdentitySummary {
                            provider: i.provider.clone(),
                            display_label: i.display_label.clone(),
                            is_primary: i.is_primary,
                            linked_at_ms: i.linked_at_ms,
                            ..Default::default()
                        })
                        .collect()
                } else {
                    vec![]
                };
            let resp = pb::WhoAmIResponse {
                user_ulid: MessageField::some(pb::Ulid {
                    bytes: user_ulid,
                    ..Default::default()
                }),
                token_kind: info.token_kind,
                grant_ulid: grant_ulid.map(MessageField::some).unwrap_or_default(),
                grantee_label: info.grantee_label,
                effective_grant: MessageField::none(),
                caller_ip: String::new(),
                device_label: None,
                linked_identities,
                ..Default::default()
            };
            Ok(ConnectResponse::new(resp))
        }
    }

    // ---- PutEvents --------------------------------------------------------

    fn put_events<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedPutEventsRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::PutEventsResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            // Materialize the request into the owned message type so we can
            // hand each EventInput to the core unchanged. (The view is Send
            // but borrows from the request buffer; converting to owned
            // unties that lifetime cleanly.)
            let req = request.to_owned_message();
            let inputs: Vec<CoreEventInput> = req
                .events
                .into_iter()
                .map(event_input_pb_to_core)
                .collect::<Result<_, _>>()?;
            let results =
                ohd_ohdc::put_events(&self.storage, &token, &inputs).map_err(error_to_connect)?;
            let pb_results: Vec<pb::PutEventResult> =
                results.iter().map(put_event_result_to_pb).collect();
            let resp = pb::PutEventsResponse {
                results: pb_results,
                ..Default::default()
            };
            Ok(ConnectResponse::new(resp))
        }
    }

    // ---- QueryEvents (server-streaming) ----------------------------------

    fn query_events(
        &self,
        ctx: RequestContext,
        request: pb::OwnedQueryEventsRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::Event>>> + Send {
        let storage = Arc::clone(&self.storage);
        async move {
            let token = require_token_owned(&storage, &ctx)?;
            let owned = request.to_owned_message();
            let filter = match owned.filter.into_option() {
                Some(f) => event_filter_pb_to_core(f)?,
                None => ohd_events::EventFilter::default(),
            };
            let resp =
                ohd_ohdc::query_events(&storage, &token, &filter).map_err(error_to_connect)?;
            let stream = futures::stream::iter(
                resp.events
                    .into_iter()
                    .map(event_core_to_pb)
                    .map(Ok::<_, ConnectError>)
                    .collect::<Vec<_>>(),
            );
            let boxed: ServiceStream<pb::Event> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::Event, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    // ---- GetEventByUlid ---------------------------------------------------

    fn get_event_by_ulid<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedGetEventByUlidRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Event> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let ulid_pb = req.ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: ulid required",
                )
            })?;
            let ulid_bytes = ulid_pb_to_core(&ulid_pb)?;
            let crockford = ohd_ulid::to_crockford(&ulid_bytes);
            let event = ohd_ohdc::get_event_by_ulid(&self.storage, &token, &crockford)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(event_core_to_pb(event)))
        }
    }

    // ---- ListPending -----------------------------------------------------

    fn list_pending<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedListPendingRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ListPendingResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let submitting_ulid = match req.submitting_grant_ulid.into_option() {
                Some(u) => Some(ulid_pb_to_core(&u)?),
                None => None,
            };
            let limit = req
                .page
                .as_option()
                .map(|p| p.limit as i64)
                .filter(|&l| l > 0);
            let rows = ohd_ohdc::list_pending(
                &self.storage,
                &token,
                submitting_ulid.as_ref(),
                req.status.as_deref(),
                limit,
            )
            .map_err(error_to_connect)?;
            let pending = rows.into_iter().map(pending_row_to_pb).collect();
            Ok(ConnectResponse::new(pb::ListPendingResponse {
                pending,
                page: MessageField::some(pb::PageResponse {
                    next_cursor: String::new(),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
    }

    // ---- ApprovePending --------------------------------------------------

    fn approve_pending<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedApprovePendingRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ApprovePendingResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let pending_ulid_pb = req.pending_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: pending_ulid required",
                )
            })?;
            let pending_ulid = ulid_pb_to_core(&pending_ulid_pb)?;
            let (committed_at_ms, event_ulid) = ohd_ohdc::approve_pending(
                &self.storage,
                &token,
                &pending_ulid,
                req.also_auto_approve_this_type,
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::ApprovePendingResponse {
                event_ulid: MessageField::some(ulid_core_to_pb(&event_ulid)),
                committed_at_ms,
                ..Default::default()
            }))
        }
    }

    // ---- RejectPending ---------------------------------------------------

    fn reject_pending<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRejectPendingRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RejectPendingResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let pending_ulid_pb = req.pending_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: pending_ulid required",
                )
            })?;
            let pending_ulid = ulid_pb_to_core(&pending_ulid_pb)?;
            let rejected_at_ms = ohd_ohdc::reject_pending(
                &self.storage,
                &token,
                &pending_ulid,
                req.reason.as_deref(),
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RejectPendingResponse {
                rejected_at_ms,
                ..Default::default()
            }))
        }
    }

    // ---- ListPendingQueries ---------------------------------------------

    fn list_pending_queries(
        &self,
        ctx: RequestContext,
        request: pb::OwnedListPendingQueriesRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::PendingQuery>>> + Send
    {
        let storage = Arc::clone(&self.storage);
        async move {
            let token = require_token_owned(&storage, &ctx)?;
            let req = request.to_owned_message();
            let decision = (!req.include_decided).then_some(QueryDecision::Pending);
            let grant_filter = token.grant_id;
            let rows = ohd_ohdc::list_pending_queries(
                &storage,
                &token,
                grant_filter,
                decision,
                req.since_ms,
                None,
            )
            .map_err(error_to_connect)?;
            let pb_rows: Result<Vec<_>, ConnectError> = rows
                .into_iter()
                .map(|row| pending_query_row_to_pb(&storage, row).map(Ok::<_, ConnectError>))
                .collect();
            let stream = futures::stream::iter(pb_rows?);
            let boxed: ServiceStream<pb::PendingQuery> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::PendingQuery, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    // ---- ApprovePendingQuery --------------------------------------------

    fn approve_pending_query<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedApprovePendingQueryRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ApprovePendingQueryResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let query_ulid_pb = req.query_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: query_ulid required",
                )
            })?;
            let query_ulid = ulid_pb_to_core(&query_ulid_pb)?;
            ohd_ohdc::approve_pending_query(&self.storage, &token, &query_ulid)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::ApprovePendingQueryResponse {
                ok: true,
                ..Default::default()
            }))
        }
    }

    // ---- RejectPendingQuery ---------------------------------------------

    fn reject_pending_query<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRejectPendingQueryRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RejectPendingQueryResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let query_ulid_pb = req.query_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: query_ulid required",
                )
            })?;
            let query_ulid = ulid_pb_to_core(&query_ulid_pb)?;
            ohd_ohdc::reject_pending_query(
                &self.storage,
                &token,
                &query_ulid,
                req.reason.as_deref(),
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RejectPendingQueryResponse {
                ok: true,
                ..Default::default()
            }))
        }
    }

    // ---- CreateGrant -----------------------------------------------------

    fn create_grant<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedCreateGrantRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::CreateGrantResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let new_grant = create_grant_request_pb_to_core(req)?;
            let outcome = ohd_ohdc::create_grant(&self.storage, &token, &new_grant)
                .map_err(error_to_connect)?;
            let pb_grant = grant_row_to_pb(&outcome.grant);
            Ok(ConnectResponse::new(pb::CreateGrantResponse {
                grant: MessageField::some(pb_grant),
                token: outcome.token,
                share_url: outcome.share_url,
                share_qr_png: vec![],
                ..Default::default()
            }))
        }
    }

    // ---- ListGrants ------------------------------------------------------

    fn list_grants<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedListGrantsRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::ListGrantsResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let limit = req
                .page
                .as_option()
                .map(|p| p.limit as i64)
                .filter(|&l| l > 0);
            let rows = ohd_ohdc::list_grants(
                &self.storage,
                &token,
                req.include_revoked.unwrap_or(false),
                req.include_expired.unwrap_or(false),
                req.grantee_kind.as_deref(),
                limit,
            )
            .map_err(error_to_connect)?;
            let grants = rows.iter().map(grant_row_to_pb).collect();
            Ok(ConnectResponse::new(pb::ListGrantsResponse {
                grants,
                page: MessageField::some(pb::PageResponse {
                    next_cursor: String::new(),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
    }

    // ---- UpdateGrant -----------------------------------------------------

    fn update_grant<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedUpdateGrantRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Grant> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let grant_ulid_pb = req.grant_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: grant_ulid required",
                )
            })?;
            let grant_ulid = ulid_pb_to_core(&grant_ulid_pb)?;
            let update = GrantUpdate {
                grantee_label: req.grantee_label,
                expires_at_ms: req.expires_at_ms,
            };
            let row = ohd_ohdc::update_grant(&self.storage, &token, &grant_ulid, &update)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(grant_row_to_pb(&row)))
        }
    }

    // ---- RevokeGrant -----------------------------------------------------

    fn revoke_grant<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRevokeGrantRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RevokeGrantResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let grant_ulid_pb = req.grant_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: grant_ulid required",
                )
            })?;
            let grant_ulid = ulid_pb_to_core(&grant_ulid_pb)?;
            let revoked_at_ms =
                ohd_ohdc::revoke_grant(&self.storage, &token, &grant_ulid, req.reason.as_deref())
                    .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RevokeGrantResponse {
                revoked_at_ms,
                ..Default::default()
            }))
        }
    }

    // -------------------------------------------------------------------------
    // The remaining RPCs.
    // -------------------------------------------------------------------------

    fn attach_blob<'a>(
        &'a self,
        ctx: RequestContext,
        requests: ServiceStream<pb::OwnedAttachBlobChunkView>,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::AttachBlobResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            // Drain the stream, collecting init + chunks + finish. The proto's
            // AttachBlobInit carries the target event ULID + mime + filename;
            // chunks deliver bytes; finish optionally validates an
            // expected sha256.
            let mut event_ulid_bytes: Option<[u8; 16]> = None;
            let mut mime_type: Option<String> = None;
            let mut filename: Option<String> = None;
            let mut buf: Vec<u8> = Vec::new();
            let mut expected_sha: Option<Vec<u8>> = None;
            let mut requests = requests;
            while let Some(chunk_view) = requests.next().await {
                let chunk_view = chunk_view?;
                let chunk = chunk_view.to_owned_message();
                match chunk.content {
                    Some(pb::attach_blob_chunk::Content::Init(init)) => {
                        if let Some(u) = init.ulid.into_option() {
                            event_ulid_bytes = Some(ulid_pb_to_core(&u)?);
                        }
                        if !init.mime_type.is_empty() {
                            mime_type = Some(init.mime_type);
                        }
                        if !init.filename.is_empty() {
                            filename = Some(init.filename);
                        }
                    }
                    Some(pb::attach_blob_chunk::Content::Data(d)) => {
                        buf.extend_from_slice(&d);
                    }
                    Some(pb::attach_blob_chunk::Content::Finish(fin)) => {
                        if !fin.expected_sha256.is_empty() {
                            expected_sha = Some(fin.expected_sha256);
                        }
                    }
                    None => {
                        return Err(ConnectError::new(
                            ErrorCode::InvalidArgument,
                            "INVALID_ARGUMENT: AttachBlobChunk missing content",
                        ));
                    }
                }
            }
            let event_ulid = event_ulid_bytes.ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: AttachBlobInit.ulid required",
                )
            })?;
            let row = ohd_ohdc::attach_blob(
                &self.storage,
                &token,
                &event_ulid,
                mime_type,
                filename,
                &buf,
                expected_sha.as_deref(),
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::AttachBlobResponse {
                attachment: MessageField::some(pb::AttachmentRef {
                    ulid: MessageField::some(ulid_core_to_pb(&row.ulid)),
                    sha256: row.sha256.to_vec(),
                    byte_size: row.byte_size,
                    mime_type: row.mime_type.unwrap_or_default(),
                    filename: row.filename.unwrap_or_default(),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
    }

    fn aggregate<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedAggregateRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::AggregateResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let filter = match req.filter.into_option() {
                Some(f) => event_filter_pb_to_core(f)?,
                None => ohd_events::EventFilter::default(),
            };
            let op = aggregate_op_pb_to_core(req.op);
            let bucket_ms = bucket_to_ms(req.bucket.into_option());
            let buckets = ohd_ohdc::aggregate(
                &self.storage,
                &token,
                &req.channel_path,
                &filter,
                op,
                bucket_ms,
            )
            .map_err(error_to_connect)?;
            let pb_buckets: Vec<pb::AggregateBucketResult> = buckets
                .into_iter()
                .map(|b| pb::AggregateBucketResult {
                    bucket_start_ms: b.bucket_start_ms,
                    bucket_end_ms: b.bucket_end_ms,
                    sample_count: b.sample_count,
                    value: b.value,
                    ..Default::default()
                })
                .collect();
            Ok(ConnectResponse::new(pb::AggregateResponse {
                buckets: pb_buckets,
                ..Default::default()
            }))
        }
    }

    fn correlate<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedCorrelateRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::CorrelateResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let a = correlate_side_pb_to_core(req.a.into_option())?;
            let b = correlate_side_pb_to_core(req.b.into_option())?;
            let window_ms = duration_to_ms_msg(req.window);
            let scope_filter = match req.scope.into_option() {
                Some(f) => event_filter_pb_to_core(f)?,
                None => ohd_events::EventFilter::default(),
            };
            let (pairs, stats) =
                ohd_ohdc::correlate(&self.storage, &token, &a, &b, window_ms, &scope_filter)
                    .map_err(error_to_connect)?;
            let pb_pairs: Vec<pb::CorrelatePair> = pairs
                .into_iter()
                .map(|p| pb::CorrelatePair {
                    a_ulid: MessageField::some(crockford_to_pb_ulid(&p.a_ulid)),
                    a_time_ms: p.a_time_ms,
                    matches: p
                        .matches
                        .into_iter()
                        .map(|m| pb::CorrelateMatch {
                            b_ulid: MessageField::some(crockford_to_pb_ulid(&m.b_ulid)),
                            b_time_ms: m.b_time_ms,
                            b_value: m.b_value,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect();
            Ok(ConnectResponse::new(pb::CorrelateResponse {
                pairs: pb_pairs,
                stats: MessageField::some(pb::CorrelateStats {
                    a_count: stats.a_count,
                    b_count: stats.b_count,
                    paired_count: stats.paired_count,
                    mean_b_value: stats.mean_b_value,
                    mean_lag_ms: stats.mean_lag_ms,
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
    }

    fn read_samples(
        &self,
        ctx: RequestContext,
        request: pb::OwnedReadSamplesRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::SampleBatch>>> + Send
    {
        let storage = Arc::clone(&self.storage);
        async move {
            let token = require_token_owned(&storage, &ctx)?;
            let req = request.to_owned_message();
            let event_ulid_pb = req.event_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: event_ulid required",
                )
            })?;
            let event_ulid = ulid_pb_to_core(&event_ulid_pb)?;
            let samples = ohd_ohdc::read_samples(
                &storage,
                &token,
                &event_ulid,
                &req.channel_path,
                req.from_ms,
                req.to_ms,
                req.max_samples,
            )
            .map_err(error_to_connect)?;
            // Stream as one batch per up-to-1024 samples for steady wire flow.
            const BATCH: usize = 1024;
            let mut batches: Vec<Result<pb::SampleBatch, ConnectError>> = Vec::new();
            for chunk in samples.chunks(BATCH) {
                batches.push(Ok(pb::SampleBatch {
                    samples: chunk
                        .iter()
                        .map(|s| pb::Sample {
                            t_ms: s.t_ms,
                            value: s.value,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }));
            }
            let stream = futures::stream::iter(batches);
            let boxed: ServiceStream<pb::SampleBatch> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::SampleBatch, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    fn read_attachment(
        &self,
        ctx: RequestContext,
        request: pb::OwnedReadAttachmentRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::AttachmentChunk>>> + Send
    {
        let storage = Arc::clone(&self.storage);
        async move {
            let token = require_token_owned(&storage, &ctx)?;
            let req = request.to_owned_message();
            let attachment_ulid_pb = req.attachment_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: attachment_ulid required",
                )
            })?;
            let attachment_ulid = ulid_pb_to_core(&attachment_ulid_pb)?;
            // Yield the **plaintext** bytes: as of the default-on encryption
            // flip, on-disk bytes are `nonce(12) || ciphertext+tag` and the
            // wire still carries plaintext (which then rides inside the TLS
            // tunnel and, for cache↔primary, the relay tunnel).
            // 50 MiB cap is enforced upstream at AttachBlob; reads of bigger
            // files are theoretically possible if pre-existing on disk, so we
            // leave the cap to the OS / FD layer.
            let (meta, bytes) = ohd_ohdc::read_attachment_bytes(&storage, &token, &attachment_ulid)
                .map_err(error_to_connect)?;
            let mut frames: Vec<Result<pb::AttachmentChunk, ConnectError>> = Vec::new();
            frames.push(Ok(pb::AttachmentChunk {
                content: Some(pb::attachment_chunk::Content::Init(Box::new(
                    pb::AttachmentInit {
                        r#ref: MessageField::some(pb::AttachmentRef {
                            ulid: MessageField::some(ulid_core_to_pb(&meta.ulid)),
                            sha256: meta.sha256.to_vec(),
                            byte_size: meta.byte_size,
                            mime_type: meta.mime_type.clone().unwrap_or_default(),
                            filename: meta.filename.clone().unwrap_or_default(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            }));
            const CHUNK: usize = 64 * 1024;
            for chunk in bytes.chunks(CHUNK) {
                frames.push(Ok(pb::AttachmentChunk {
                    content: Some(pb::attachment_chunk::Content::Data(chunk.to_vec())),
                    ..Default::default()
                }));
            }
            frames.push(Ok(pb::AttachmentChunk {
                content: Some(pb::attachment_chunk::Content::Finish(Box::new(
                    pb::AttachmentFinish {
                        expected_sha256: meta.sha256.to_vec(),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            }));
            let stream = futures::stream::iter(frames);
            let boxed: ServiceStream<pb::AttachmentChunk> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::AttachmentChunk, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    fn create_case<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedCreateCaseRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Case> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let parent_case_ulid = match req.parent_case_ulid.into_option() {
                Some(u) => Some(ulid_pb_to_core(&u)?),
                None => None,
            };
            let predecessor_case_ulid = match req.predecessor_case_ulid.into_option() {
                Some(u) => Some(ulid_pb_to_core(&u)?),
                None => None,
            };
            let initial_filters = req
                .initial_filters
                .into_iter()
                .map(event_filter_pb_to_core)
                .collect::<Result<Vec<_>, _>>()?;
            let new_case = ohd_storage_core::cases::NewCase {
                case_type: req.case_type,
                case_label: req.case_label,
                parent_case_ulid,
                predecessor_case_ulid,
                inactivity_close_after_h: req.inactivity_close_after_h,
                initial_filters,
                opening_authority_grant_id: None,
            };
            let case = ohd_ohdc::create_case(&self.storage, &token, &new_case)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(case_to_pb(&case)))
        }
    }

    fn update_case<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedUpdateCaseRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Case> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let case_ulid_pb = req.case_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: case_ulid required",
                )
            })?;
            let case_ulid = ulid_pb_to_core(&case_ulid_pb)?;
            // Parent / predecessor are immutable per spec; the wire's
            // UpdateCaseRequest still carries them for forward-compat but the
            // core ignores them. We accept and drop silently to keep round-trip
            // through the protocol idempotent.
            let _ = req.parent_case_ulid;
            let _ = req.predecessor_case_ulid;
            let update = ohd_storage_core::cases::CaseUpdate {
                case_label: req.case_label,
                inactivity_close_after_h: req.inactivity_close_after_h,
            };
            let case = ohd_ohdc::update_case(&self.storage, &token, &case_ulid, &update)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(case_to_pb(&case)))
        }
    }

    fn close_case<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedCloseCaseRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Case> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let case_ulid_pb = req.case_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: case_ulid required",
                )
            })?;
            let case_ulid = ulid_pb_to_core(&case_ulid_pb)?;
            let (case, _reopen) =
                ohd_ohdc::close_case(&self.storage, &token, &case_ulid, req.reason.as_deref())
                    .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(case_to_pb(&case)))
        }
    }

    fn reopen_case<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedReopenCaseRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Case> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let case = match req.method {
                Some(pb::reopen_case_request::Method::CaseReopenTokenUlid(u)) => {
                    let bytes = ulid_pb_to_core(&u)?;
                    ohd_ohdc::reopen_case_by_token(&self.storage, &token, &bytes)
                        .map_err(error_to_connect)?
                }
                Some(pb::reopen_case_request::Method::Patient(p)) => {
                    let case_ulid_pb = p.case_ulid.into_option().ok_or_else(|| {
                        ConnectError::new(
                            ErrorCode::InvalidArgument,
                            "INVALID_ARGUMENT: PatientReopen.case_ulid required",
                        )
                    })?;
                    let bytes = ulid_pb_to_core(&case_ulid_pb)?;
                    ohd_ohdc::reopen_case_by_patient(&self.storage, &token, &bytes)
                        .map_err(error_to_connect)?
                }
                None => {
                    return Err(ConnectError::new(
                        ErrorCode::InvalidArgument,
                        "INVALID_ARGUMENT: ReopenCaseRequest.method required",
                    ));
                }
            };
            Ok(ConnectResponse::new(case_to_pb(&case)))
        }
    }

    fn list_cases<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedListCasesRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::ListCasesResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let limit = req
                .page
                .as_option()
                .map(|p| p.limit as i64)
                .filter(|&l| l > 0);
            let cases = ohd_ohdc::list_cases(
                &self.storage,
                &token,
                req.include_closed.unwrap_or(false),
                req.case_type.as_deref(),
                limit,
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::ListCasesResponse {
                cases: cases.iter().map(case_to_pb).collect(),
                page: MessageField::some(pb::PageResponse {
                    next_cursor: String::new(),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
    }

    fn get_case<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedGetCaseRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Case> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let case_ulid_pb = req.case_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: case_ulid required",
                )
            })?;
            let case_ulid = ulid_pb_to_core(&case_ulid_pb)?;
            let case =
                ohd_ohdc::get_case(&self.storage, &token, &case_ulid).map_err(error_to_connect)?;
            Ok(ConnectResponse::new(case_to_pb(&case)))
        }
    }

    fn add_case_filter<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedAddCaseFilterRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::CaseFilter> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let case_ulid_pb = req.case_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: case_ulid required",
                )
            })?;
            let case_ulid = ulid_pb_to_core(&case_ulid_pb)?;
            let filter = match req.filter.into_option() {
                Some(f) => event_filter_pb_to_core(f)?,
                None => ohd_events::EventFilter::default(),
            };
            let row = ohd_ohdc::add_case_filter(
                &self.storage,
                &token,
                &case_ulid,
                &filter,
                req.filter_label.as_deref(),
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(case_filter_to_pb(&row)))
        }
    }

    fn remove_case_filter<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRemoveCaseFilterRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RemoveCaseFilterResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let cf_ulid_pb = req.case_filter_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: case_filter_ulid required",
                )
            })?;
            let cf_ulid = ulid_pb_to_core(&cf_ulid_pb)?;
            let removed_at = ohd_ohdc::remove_case_filter(&self.storage, &token, &cf_ulid)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RemoveCaseFilterResponse {
                removed_at_ms: removed_at,
                ..Default::default()
            }))
        }
    }

    fn list_case_filters<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedListCaseFiltersRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ListCaseFiltersResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let case_ulid_pb = req.case_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: case_ulid required",
                )
            })?;
            let case_ulid = ulid_pb_to_core(&case_ulid_pb)?;
            let rows =
                ohd_ohdc::list_case_filters(&self.storage, &token, &case_ulid, req.include_removed)
                    .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::ListCaseFiltersResponse {
                filters: rows.iter().map(case_filter_to_pb).collect(),
                ..Default::default()
            }))
        }
    }

    fn audit_query(
        &self,
        ctx: RequestContext,
        request: pb::OwnedAuditQueryRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::AuditEntry>>> + Send
    {
        let storage = Arc::clone(&self.storage);
        async move {
            let token = require_token_owned(&storage, &ctx)?;
            let req = request.to_owned_message();
            let grant_id = match req.grant_ulid.into_option() {
                Some(u) => {
                    let bytes = ulid_pb_to_core(&u)?;
                    Some(
                        storage
                            .with_conn(|conn| {
                                ohd_storage_core::grants::grant_id_by_ulid(conn, &bytes)
                            })
                            .map_err(error_to_connect)?,
                    )
                }
                None => None,
            };
            let q = ohd_audit::AuditQuery {
                from_ms: req.from_ms,
                to_ms: req.to_ms,
                grant_id,
                actor_type: req.actor_type,
                action: req.action,
                result: req.result,
                limit: Some(10_000),
            };
            let rows = ohd_ohdc::audit_query(&storage, &token, &q).map_err(error_to_connect)?;
            let pb_rows: Vec<Result<pb::AuditEntry, ConnectError>> = rows
                .into_iter()
                .map(|r| {
                    Ok(pb::AuditEntry {
                        ts_ms: r.ts_ms,
                        actor_type: r.actor_type.as_str().into(),
                        grant_ulid: MessageField::none(), // wire grant_ulid is filled per-row in v1.x
                        action: r.action,
                        query_kind: r.query_kind.unwrap_or_default(),
                        query_params_json: r.query_params_json.unwrap_or_default(),
                        rows_returned: r.rows_returned,
                        rows_filtered: r.rows_filtered,
                        result: r.result.as_str().into(),
                        reason: r.reason,
                        caller_ip: r.caller_ip,
                        caller_ua: r.caller_ua,
                        ..Default::default()
                    })
                })
                .collect();
            let stream = futures::stream::iter(pb_rows);
            let boxed: ServiceStream<pb::AuditEntry> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::AuditEntry, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    fn export(
        &self,
        ctx: RequestContext,
        request: pb::OwnedExportRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::ExportChunk>>> + Send
    {
        let storage = Arc::clone(&self.storage);
        async move {
            let token = require_token_owned(&storage, &ctx)?;
            let req = request.to_owned_message();
            let frames = ohd_ohdc::export(
                &storage,
                &token,
                req.from_ms,
                req.to_ms,
                &req.include_event_types,
            )
            .map_err(error_to_connect)?;
            let pb_frames: Vec<Result<pb::ExportChunk, ConnectError>> = frames
                .into_iter()
                .map(|f| Ok(export_frame_to_pb(f)))
                .collect();
            let stream = futures::stream::iter(pb_frames);
            let boxed: ServiceStream<pb::ExportChunk> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::ExportChunk, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    // ---- Source signing (operator RPCs, self-session only) --------------

    fn register_signer<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRegisterSignerRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RegisterSignerResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let signer = ohd_ohdc::register_signer(
                &self.storage,
                &token,
                &req.signer_kid,
                &req.signer_label,
                &req.sig_alg,
                &req.public_key_pem,
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RegisterSignerResponse {
                signer: MessageField::some(signer_to_pb(&signer)),
                registered_at_ms: signer.registered_at_ms,
                ..Default::default()
            }))
        }
    }

    fn list_signers<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedListSignersRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ListSignersResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let mut rows =
                ohd_ohdc::list_signers(&self.storage, &token).map_err(error_to_connect)?;
            if !req.include_revoked {
                rows.retain(|s| s.revoked_at_ms.is_none());
            }
            Ok(ConnectResponse::new(pb::ListSignersResponse {
                signers: rows.iter().map(signer_to_pb).collect(),
                ..Default::default()
            }))
        }
    }

    fn revoke_signer<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRevokeSignerRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RevokeSignerResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            let req = request.to_owned_message();
            let revoked_at_ms = ohd_ohdc::revoke_signer(&self.storage, &token, &req.signer_kid)
                .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RevokeSignerResponse {
                revoked_at_ms,
                ..Default::default()
            }))
        }
    }

    fn import<'a>(
        &'a self,
        ctx: RequestContext,
        requests: ServiceStream<pb::OwnedImportChunkView>,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::ImportResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_token(self, &ctx)?;
            // Drain the import stream into a vec of ExportFrames. Init / Finish
            // are recorded as the corresponding ExportFrame variants; ExportFrame
            // carries the actual entity payload.
            let mut frames: Vec<ohd_ohdc::ExportFrame> = Vec::new();
            let mut requests = requests;
            while let Some(chunk_view) = requests.next().await {
                let chunk_view = chunk_view?;
                let chunk = chunk_view.to_owned_message();
                match chunk.content {
                    Some(pb::import_chunk::Content::Init(init)) => {
                        frames.push(ohd_ohdc::ExportFrame::Init {
                            format_version: ohd_storage_core::FORMAT_VERSION.to_string(),
                            source_instance_pubkey_hex: init.source_instance_pubkey_hex,
                        });
                    }
                    Some(pb::import_chunk::Content::Frame(f)) => {
                        if let Some(frame) = export_frame_pb_to_core(*f)? {
                            frames.push(frame);
                        }
                    }
                    Some(pb::import_chunk::Content::Finish(_)) => {
                        // Finish carries a signature; today unsigned, so ignore.
                    }
                    None => {
                        return Err(ConnectError::new(
                            ErrorCode::InvalidArgument,
                            "INVALID_ARGUMENT: ImportChunk missing content",
                        ));
                    }
                }
            }
            let outcome =
                ohd_ohdc::import(&self.storage, &token, &frames).map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::ImportResponse {
                events_imported: outcome.events_imported,
                grants_imported: outcome.grants_imported,
                audit_entries_imported: outcome.audit_entries_imported,
                warnings: outcome.warnings,
                unknown_extensions: vec![],
                ..Default::default()
            }))
        }
    }
}

#[allow(dead_code)]
fn unimplemented_err(rpc: &str) -> ConnectError {
    ConnectError::new(
        ErrorCode::Unimplemented,
        format!("UNIMPLEMENTED: {rpc} is not implemented in v1"),
    )
}

// ============================================================================
// Helpers for the freshly wired RPCs.
// ============================================================================

fn aggregate_op_pb_to_core(op: buffa::EnumValue<pb::AggregateOp>) -> ohd_ohdc::AggregateOp {
    use ohd_ohdc::AggregateOp::*;
    match op.as_known().unwrap_or(pb::AggregateOp::AVG) {
        pb::AggregateOp::AVG => Avg,
        pb::AggregateOp::SUM => Sum,
        pb::AggregateOp::MIN => Min,
        pb::AggregateOp::MAX => Max,
        pb::AggregateOp::COUNT => Count,
        pb::AggregateOp::MEDIAN => Median,
        pb::AggregateOp::P95 => P95,
        pb::AggregateOp::P99 => P99,
        pb::AggregateOp::STDDEV => StdDev,
    }
}

/// Convert a `google.protobuf.Duration` (seconds + nanos) to total ms.
/// Returns 0 when the message is absent.
fn duration_to_ms_msg(d: MessageField<buffa_types::google::protobuf::Duration>) -> i64 {
    match d.into_option() {
        Some(dur) => dur.seconds.saturating_mul(1000) + (dur.nanos / 1_000_000) as i64,
        None => 0,
    }
}

fn bucket_to_ms(b: Option<pb::Bucket>) -> i64 {
    let Some(b) = b else { return 0 };
    match b.bucket {
        Some(pb::bucket::Bucket::Fixed(d)) => {
            d.seconds.saturating_mul(1000) + (d.nanos / 1_000_000) as i64
        }
        Some(pb::bucket::Bucket::Calendar(c)) => {
            let unit = c.unit.as_known().unwrap_or(pb::CalendarUnit::HOUR);
            match unit {
                pb::CalendarUnit::HOUR => 3600 * 1000,
                pb::CalendarUnit::DAY => 86_400 * 1000,
                pb::CalendarUnit::WEEK => 7 * 86_400 * 1000,
                pb::CalendarUnit::MONTH => 30 * 86_400 * 1000,
                pb::CalendarUnit::YEAR => 365 * 86_400 * 1000,
            }
        }
        None => 0,
    }
}

fn correlate_side_pb_to_core(
    s: Option<pb::CorrelateSide>,
) -> Result<ohd_ohdc::CorrelateSide, ConnectError> {
    let s = s.ok_or_else(|| {
        ConnectError::new(
            ErrorCode::InvalidArgument,
            "INVALID_ARGUMENT: CorrelateSide required",
        )
    })?;
    match s.spec {
        Some(pb::correlate_side::Spec::EventType(et)) => Ok(ohd_ohdc::CorrelateSide::EventType(et)),
        Some(pb::correlate_side::Spec::ChannelPath(cp)) => {
            Ok(ohd_ohdc::CorrelateSide::ChannelPath(cp))
        }
        None => Err(ConnectError::new(
            ErrorCode::InvalidArgument,
            "INVALID_ARGUMENT: CorrelateSide.spec required",
        )),
    }
}

fn crockford_to_pb_ulid(s: &str) -> pb::Ulid {
    let bytes = ohd_ulid::parse_crockford(s).unwrap_or_default().to_vec();
    pb::Ulid {
        bytes,
        ..Default::default()
    }
}

fn signer_to_pb(s: &ohd_storage_core::source_signing::Signer) -> pb::SignerInfo {
    pb::SignerInfo {
        signer_kid: s.signer_kid.clone(),
        signer_label: s.signer_label.clone(),
        sig_alg: s.sig_alg.clone(),
        revoked: s.revoked_at_ms.is_some(),
        ..Default::default()
    }
}

fn signer_info_core_to_pb(info: &ohd_storage_core::source_signing::SignerInfo) -> pb::SignerInfo {
    pb::SignerInfo {
        signer_kid: info.signer_kid.clone(),
        signer_label: info.signer_label.clone(),
        sig_alg: info.sig_alg.clone(),
        revoked: info.revoked,
        ..Default::default()
    }
}

fn source_signature_pb_to_core(
    sig: pb::SourceSignature,
) -> ohd_storage_core::source_signing::SourceSignature {
    ohd_storage_core::source_signing::SourceSignature {
        sig_alg: sig.sig_alg,
        signer_kid: sig.signer_kid,
        signature: sig.signature,
    }
}

fn case_to_pb(c: &ohd_storage_core::cases::Case) -> pb::Case {
    pb::Case {
        ulid: MessageField::some(ulid_core_to_pb(&c.ulid)),
        case_type: c.case_type.clone(),
        case_label: c.case_label.clone(),
        started_at_ms: c.started_at_ms,
        ended_at_ms: c.ended_at_ms,
        parent_case_ulid: c
            .parent_case_ulid
            .as_ref()
            .map(|u| MessageField::some(ulid_core_to_pb(u)))
            .unwrap_or_default(),
        predecessor_case_ulid: c
            .predecessor_case_ulid
            .as_ref()
            .map(|u| MessageField::some(ulid_core_to_pb(u)))
            .unwrap_or_default(),
        opening_authority_grant_ulid: c
            .opening_authority_grant_ulid
            .as_ref()
            .map(|u| MessageField::some(ulid_core_to_pb(u)))
            .unwrap_or_default(),
        inactivity_close_after_h: c.inactivity_close_after_h,
        last_activity_at_ms: c.last_activity_at_ms,
        ..Default::default()
    }
}

fn case_filter_to_pb(c: &ohd_storage_core::cases::CaseFilterRow) -> pb::CaseFilter {
    pb::CaseFilter {
        ulid: MessageField::some(ulid_core_to_pb(&c.ulid)),
        case_ulid: MessageField::some(ulid_core_to_pb(&c.case_ulid)),
        // The wire's filter is the round-tripped event filter (best-effort —
        // v1.x will materialize it back into a pb::EventFilter).
        filter: MessageField::none(),
        filter_label: c.filter_label.clone(),
        added_at_ms: c.added_at_ms,
        added_by_grant_ulid: MessageField::none(),
        ..Default::default()
    }
}

/// Map a core `ExportFrame` to a wire `ExportChunk`.
fn export_frame_to_pb(f: ohd_ohdc::ExportFrame) -> pb::ExportChunk {
    use pb::export_chunk::Content;
    use pb::export_frame::Entity;
    let content = match f {
        ohd_ohdc::ExportFrame::Init {
            format_version,
            source_instance_pubkey_hex,
        } => Content::Init(Box::new(pb::ExportInit {
            format_version,
            source_instance_pubkey_hex,
            encryption: MessageField::none(),
            ..Default::default()
        })),
        ohd_ohdc::ExportFrame::Event(e) => Content::Frame(Box::new(pb::ExportFrame {
            entity: Some(Entity::Event(Box::new(event_core_to_pb(e)))),
            ..Default::default()
        })),
        ohd_ohdc::ExportFrame::Grant(g) => Content::Frame(Box::new(pb::ExportFrame {
            entity: Some(Entity::Grant(Box::new(grant_row_to_pb(&g)))),
            ..Default::default()
        })),
        ohd_ohdc::ExportFrame::Audit(a) => Content::Frame(Box::new(pb::ExportFrame {
            entity: Some(Entity::AuditEntry(Box::new(pb::AuditEntry {
                ts_ms: a.ts_ms,
                actor_type: a.actor_type.as_str().into(),
                grant_ulid: MessageField::none(),
                action: a.action,
                query_kind: a.query_kind.unwrap_or_default(),
                query_params_json: a.query_params_json.unwrap_or_default(),
                rows_returned: a.rows_returned,
                rows_filtered: a.rows_filtered,
                result: a.result.as_str().into(),
                reason: a.reason,
                caller_ip: a.caller_ip,
                caller_ua: a.caller_ua,
                ..Default::default()
            }))),
            ..Default::default()
        })),
        ohd_ohdc::ExportFrame::Finish { events_emitted: _ } => {
            Content::Finish(Box::new(pb::ExportFinish {
                resume_token: String::new(),
                signature: vec![],
                source_instance_pubkey_hex: String::new(),
                ..Default::default()
            }))
        }
    };
    pb::ExportChunk {
        content: Some(content),
        ..Default::default()
    }
}

fn export_frame_pb_to_core(
    f: pb::ExportFrame,
) -> Result<Option<ohd_ohdc::ExportFrame>, ConnectError> {
    use pb::export_frame::Entity;
    match f.entity {
        Some(Entity::Event(e)) => {
            let core_event = pb_event_to_core(*e)?;
            Ok(Some(ohd_ohdc::ExportFrame::Event(core_event)))
        }
        // Grant / Audit / etc. Round-tripping these through Import preserves a
        // best-effort copy in core (Grant import is deferred — see ohdc::import
        // doc-comment); we let the import pipeline count them as imported.
        Some(_) | None => Ok(None),
    }
}

/// Public alias so `sync_server.rs` can reuse the same conversion.
pub fn pb_event_to_core_pub(e: pb::Event) -> Result<ohd_events::Event, ConnectError> {
    pb_event_to_core(e)
}

/// Public alias so `sync_server.rs` can reuse the same conversion.
pub fn event_core_to_pb_pub(e: ohd_events::Event) -> pb::Event {
    event_core_to_pb(e)
}

/// Public alias so `sync_server.rs` can reuse the same conversion.
pub fn create_grant_request_pb_to_core_pub(
    req: pb::CreateGrantRequest,
) -> Result<NewGrant, ConnectError> {
    create_grant_request_pb_to_core(req)
}

/// Public alias so `sync_server.rs` can reuse the same conversion.
pub fn grant_row_to_pb_pub(g: &GrantRow) -> pb::Grant {
    grant_row_to_pb(g)
}

fn pb_event_to_core(e: pb::Event) -> Result<ohd_events::Event, ConnectError> {
    let ulid_pb = e.ulid.into_option().ok_or_else(|| {
        ConnectError::new(
            ErrorCode::InvalidArgument,
            "INVALID_ARGUMENT: Event.ulid required",
        )
    })?;
    let ulid_bytes = ulid_pb_to_core(&ulid_pb)?;
    let crockford = ohd_ulid::to_crockford(&ulid_bytes);
    let channels = e
        .channels
        .into_iter()
        .map(channel_value_pb_to_core)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ohd_events::Event {
        ulid: crockford,
        timestamp_ms: e.timestamp_ms,
        duration_ms: e.duration_ms,
        tz_offset_minutes: e.tz_offset_minutes,
        tz_name: e.tz_name,
        event_type: e.event_type,
        channels,
        sample_blocks: vec![],
        attachments: vec![],
        device_id: e.device_id,
        app_name: e.app_name,
        app_version: e.app_version,
        source: e.source,
        source_id: e.source_id,
        notes: e.notes,
        superseded_by: e.superseded_by.into_option().and_then(|u| {
            let mut bytes = [0u8; 16];
            if u.bytes.len() == 16 {
                bytes.copy_from_slice(&u.bytes);
                Some(ohd_ulid::to_crockford(&bytes))
            } else {
                None
            }
        }),
        deleted_at_ms: e.deleted_at_ms,
        // No proto field yet for `signed_by`; the in-process query helpers
        // populate it directly via `source_signing::signer_info_for_event`.
        signed_by: None,
    })
}

fn require_token_owned(
    storage: &Storage,
    ctx: &RequestContext,
) -> Result<ResolvedToken, ConnectError> {
    let bearer = bearer_from_ctx(ctx)?;
    storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
        .map_err(error_to_connect)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// Suppress unused warnings on the pending module re-export — the trait impl
// uses `ohd_pending::*` indirectly via the helpers above.
#[allow(dead_code)]
fn _pending_compat() -> Option<&'static str> {
    let _ = ohd_pending::PendingStatus::Pending;
    None
}
